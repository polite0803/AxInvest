// i18n-exempt: 无硬编码文案，全部走 t()
/**
 * HorizonDecisionStrip —— 四周期（超短/短线/中期/长期）逐档信息条。
 *
 * 为什么需要它：阶段2 起一次分析产四档独立决策（`decisionsByHorizon`），但此前**只有**
 * `DecisionBanner` 的专业模式按档展示；而工作区壳层下 `DecisionBanner` 走的是嵌入分支
 * （只渲染极简工具栏，四周期区块全在 `!embeddedInWorkspace` 里），决策 Tab 走
 * `DecisionComparisonPanel`（从未读该字段），风险 / 辩论卡片只读各自的单份产物
 * ⇒ 用户在工作区里三处都看不到四档。
 *
 * 两条呈现轴（不是同一份数据的两种摆法，由 `mode` 选一）：
 *   · `mode="decision"` —— 该档的最终 `action` + 建议仓位（阶段2 四周期独立决策）；
 *   · `mode="risk"`     —— 该档的**按档风险档** `riskCategory`（v140 起由分支行带回，
 *                          产端 = 子模板内的 `cls-risk-level-<档>`）。
 *
 * 缺席规矩（全仓一致）：某档缺该轴数据就**不渲染那一格**（不拿主档顶替、不补「—」装作
 * 有格）；四档全缺则整条不渲染。档位键表与显示名分别复用 `HORIZON_CAMEL_TO_SNAKE` /
 * `HORIZON_T_SUFFIX`（单一真相源），不在此另立一份四档表。
 */
import {
  getActionColor,
  getActionTKey,
  getRiskColor,
  getRiskTKey,
  HORIZON_CAMEL_TO_SNAKE,
  HORIZON_T_SUFFIX,
} from "@/lib/stock-analysis-utils";
import type { DecisionsByHorizon, HorizonDecision } from "@/types";
import { Tag } from "antd";
import { useMemo } from "react";
import { useTranslation } from "react-i18next";

export type HorizonStripMode = "decision" | "risk";

interface HorizonDecisionStripProps {
  decisions?: DecisionsByHorizon | null;
  mode: HorizonStripMode;
  /** 容器 test id；不传则不加属性（默认不污染 DOM） */
  testId?: string;
}

export function HorizonDecisionStrip({ decisions, mode, testId }: HorizonDecisionStripProps) {
  const { t } = useTranslation();

  const entries = useMemo(() => {
    if (!decisions) { return []; }
    const out: Array<{ key: string; tierLabel: string; d: HorizonDecision }> = [];
    for (const [camel, snake] of Object.entries(HORIZON_CAMEL_TO_SNAKE)) {
      const d = decisions[camel as keyof DecisionsByHorizon];
      if (!d) { continue; }
      const suffix = HORIZON_T_SUFFIX[snake];
      if (!suffix) { continue; }
      // 缺席规矩：该轴没有值 ⇒ 不列这一档
      if (mode === "decision" ? !d.action : !d.riskCategory) { continue; }
      out.push({ key: camel, tierLabel: t(`stockAnalysis.timeHorizon${suffix}`), d });
    }
    return out;
  }, [decisions, mode, t]);

  if (entries.length === 0) { return null; }

  return (
    <div
      data-testid={testId}
      className="flex items-center gap-1.5 flex-wrap text-sm"
      style={{ color: "var(--muted)" }}
    >
      <span>{t("stockAnalysis.horizonOverview")}</span>
      {entries.map(({ key, tierLabel, d }) => (
        <span
          key={key}
          className="flex items-center gap-1 px-1.5 py-0.5 rounded"
          style={{ background: "var(--surface)", border: "1px solid var(--border)" }}
        >
          <span className="font-medium" style={{ color: "var(--color-text-secondary)" }}>
            {tierLabel}
          </span>
          {mode === "decision"
            ? (
              <>
                <Tag color={getActionColor(d.action)} style={{ margin: 0 }}>
                  {t(getActionTKey(d.action))}
                </Tag>
                {typeof d.positionPct === "number" && <span className="font-mono">{d.positionPct}%</span>}
              </>
            )
            : (
              <span className="font-mono font-medium" style={{ color: getRiskColor(d.riskCategory ?? "") }}>
                {t(getRiskTKey(d.riskCategory ?? ""))}
              </span>
            )}
        </span>
      ))}
    </div>
  );
}
