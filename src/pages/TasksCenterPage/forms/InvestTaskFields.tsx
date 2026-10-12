// SPDX-License-Identifier: AGPL-3.0-only

import { Form, Input, InputNumber, Select, Slider } from "antd";
import { useTranslation } from "react-i18next";
import type { InvestTaskKind } from "./investTaskDefs";

/**
 * 投资任务业务参数字段（不含 cron —— cron 由 Modal 主体统一渲染 CronScheduleBuilder）。
 * 必须置于 antd `Form` 上下文内；字段名与 investTaskDefs 的 initialValues / buildArgs 对齐。
 *
 * 文案策略：业务术语一律复用旧设置页已有 i18n 键（stockAnalysis.scheduledAnalysis.* /
 * settings.scheduled-recommendation.* / serenityPanel.*），仅结构性 label 用
 * tasksCenter.create.field.* 新键。
 */

const REFLECTION_PERIOD_OPTIONS = [
  { value: "ultra_short", labelKey: "stockAnalysis.scheduledAnalysis.period.ultraShort" },
  { value: "short", labelKey: "stockAnalysis.scheduledAnalysis.period.short" },
  { value: "mid", labelKey: "stockAnalysis.scheduledAnalysis.period.mid" },
  { value: "long", labelKey: "stockAnalysis.scheduledAnalysis.period.long" },
];

const RECO_PERIOD_OPTIONS = [
  { value: "ultra_short", labelKey: "settings.scheduled-recommendation.period-ultra-short" },
  { value: "short", labelKey: "settings.scheduled-recommendation.period-short" },
  { value: "mid", labelKey: "settings.scheduled-recommendation.period-mid" },
  { value: "long", labelKey: "settings.scheduled-recommendation.period-long" },
];

/** T+N 验证窗口候选（与后端 RunDecisionBacktestRequest 默认 [5,20,60] 一致） */
const BACKTEST_WINDOWS = [5, 20, 60];

export function InvestTaskFields({ kind }: { kind: InvestTaskKind }) {
  const { t } = useTranslation();

  if (kind === "stock-analysis") {
    return (
      <div className="grid grid-cols-1 md:grid-cols-2 gap-x-4">
        <Form.Item
          name="stockCode"
          // 旧设置页只有 placeholder 文案，语义即 label，直接复用
          label={t("stockAnalysis.scheduledAnalysis.placeholder.stockCode")}
          rules={[{ required: true, message: t("common.required") }]}
        >
          <Input placeholder={t("stockAnalysis.scheduledAnalysis.placeholder.stockCode")} />
        </Form.Item>
        <Form.Item
          name="stockName"
          label={t("stockAnalysis.scheduledAnalysis.placeholder.stockName")}
          rules={[{ required: true, message: t("common.required") }]}
        >
          <Input placeholder={t("stockAnalysis.scheduledAnalysis.placeholder.stockName")} />
        </Form.Item>
      </div>
    );
  }

  if (kind === "watchlist-scan") {
    // 无业务参数：到点遍历自选股逐只分析
    return null;
  }

  if (kind === "pool-scan") {
    return (
      <div className="grid grid-cols-1 md:grid-cols-3 gap-x-4">
        <Form.Item name="lookbackDays" label={t("tasksCenter.create.field.lookbackDays")}>
          <InputNumber min={0} max={3650} className="!w-full" placeholder={t("tasksCenter.create.field.optional")} />
        </Form.Item>
        <Form.Item name="maxStocks" label={t("tasksCenter.create.field.maxStocks")}>
          <InputNumber min={0} max={5000} className="!w-full" placeholder={t("tasksCenter.create.field.optional")} />
        </Form.Item>
        <Form.Item name="minConfidence" label={t("tasksCenter.create.field.minConfidence")}>
          <InputNumber min={0} max={100} className="!w-full" placeholder={t("tasksCenter.create.field.optional")} />
        </Form.Item>
      </div>
    );
  }

  if (kind === "trend-screening") {
    return (
      <Form.Item
        name="chain"
        label={t("tasksCenter.create.field.chain")}
        rules={[{ required: true, message: t("common.required") }]}
      >
        <Select
          options={[
            { value: "serenity-screening", label: t("serenityPanel.run") },
            { value: "serenity-screening-fast", label: t("serenityPanel.fastRun") },
          ]}
        />
      </Form.Item>
    );
  }

  if (kind === "batch-reflection") {
    return (
      <Form.Item name="period" label={t("tasksCenter.create.field.period")}>
        <Select options={REFLECTION_PERIOD_OPTIONS.map((o) => ({ value: o.value, label: t(o.labelKey) }))} />
      </Form.Item>
    );
  }

  if (kind === "decision-backtest") {
    return (
      <div className="grid grid-cols-1 md:grid-cols-2 gap-x-4">
        <Form.Item name="windows" label={t("tasksCenter.create.field.windows")}>
          <Select
            mode="multiple"
            options={BACKTEST_WINDOWS.map((n) => ({
              value: n,
              label: t("stockAnalysis.scheduledAnalysis.decisionBacktestWindow", { n }),
            }))}
          />
        </Form.Item>
        <Form.Item name="period" label={t("tasksCenter.create.field.period")}>
          <Select
            options={[
              { value: "all", label: t("stockAnalysis.scheduledAnalysis.decisionBacktestPeriodAll") },
              ...REFLECTION_PERIOD_OPTIONS
                .filter((o) => o.value !== "ultra_short")
                .map((o) => ({ value: o.value, label: t(o.labelKey) })),
            ]}
          />
        </Form.Item>
        <Form.Item name="maxPicks" label={t("stockAnalysis.scheduledAnalysis.decisionBacktestMaxPicks")}>
          <InputNumber min={1} max={2000} step={50} className="!w-full" />
        </Form.Item>
      </div>
    );
  }

  // stock-recommendation
  return (
    <div className="grid grid-cols-1 md:grid-cols-2 gap-x-4">
      <Form.Item
        name="name"
        label={t("settings.scheduled-recommendation.label-task-name")}
        rules={[{ required: true, max: 64 }]}
      >
        <Input placeholder={t("settings.scheduled-recommendation.placeholder-task-name")} />
      </Form.Item>
      <Form.Item name="periods" label={t("settings.scheduled-recommendation.label-periods")}>
        <Select
          mode="multiple"
          options={RECO_PERIOD_OPTIONS.map((o) => ({ value: o.value, label: t(o.labelKey) }))}
          placeholder={t("settings.scheduled-recommendation.placeholder-periods")}
          maxTagCount="responsive"
        />
      </Form.Item>
      <Form.Item name="minConfidence" label={t("settings.scheduled-recommendation.label-min-confidence")}>
        <Slider min={0} max={100} marks={{ 0: "0", 50: "50", 80: "80", 100: "100" }} />
      </Form.Item>
      <Form.Item name="topN" label={t("settings.scheduled-recommendation.label-top-n")}>
        <InputNumber min={1} max={20} className="!w-full" />
      </Form.Item>
    </div>
  );
}
