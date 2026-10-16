// i18n-exempt: 无硬编码文案，全部走 t()
/**
 * LlmHorizonDecisionStrip —— LLM（trader）自报的**逐档决策**条。
 *
 * 为什么单独一个组件、不塞进 `HorizonDecisionStrip`：两者的**缺席规矩相反**。
 *   · 公式侧那条（`mode="decision"|"risk"`）是「该档缺该轴数据 ⇒ 不渲染那一格，四档全缺 ⇒ 整条不渲染」；
 *   · LLM 侧必须**四格齐出**（拍板 2 的批注：「必须确保四档完整解析并展示」），
 *     且四种「没有」各占一句 —— 混进同一个组件迟早会让另一侧的隐藏规则悄悄生效，
 *     把「该档没给」显示成「没有这一档」。
 * 档位键表与显示名仍复用 `HORIZON_CAMEL_TO_SNAKE` / `HORIZON_T_SUFFIX`（单一真相源），
 * 不在此另立四档表；状态判据不在这里算，走 `readLlmHorizonDecisions`（其正主是 Rust 的代际标记）。
 *
 * ⚠ 与公式侧并列时两侧仓位**不是同一个口径**：LLM 侧不含档内风控否决与仓位上限，
 * 所以本条只摆 LLM 自己的结论，不与之求差、不求平均（逐档一致率是 B1 的活）。
 */
import { readLlmHorizonDecisions } from "@/lib/agentOutput";
import { getActionColor, getActionTKey, HORIZON_CAMEL_TO_SNAKE, HORIZON_T_SUFFIX } from "@/lib/stock-analysis-utils";
import type { LlmHorizonDecision } from "@/types";
import { Tag } from "antd";
import { useTranslation } from "react-i18next";

interface LlmHorizonDecisionStripProps {
  /** `stock_analyses.llm_decision_json` 原文（不在这里做任何键归一） */
  llmDecisionJson: string | null;
  /** 容器 test id；不传则不加属性 */
  testId?: string;
}

export function LlmHorizonDecisionStrip({ llmDecisionJson, testId }: LlmHorizonDecisionStripProps) {
  const { t } = useTranslation();
  const view = readLlmHorizonDecisions(llmDecisionJson);

  if (view.state === "absent") { return null; }

  // 两种「整份没有」各占一句，不与逐档缺席共用文案，也不压成四个「未给出」
  if (view.state === "parseFailed" || view.state === "preContract") {
    return (
      <div
        data-testid={testId}
        data-state={view.state}
        className="text-sm"
        style={{ color: "var(--muted)" }}
      >
        {t(
          view.state === "parseFailed"
            ? "stockAnalysis.llmHorizonParseFailed"
            : "stockAnalysis.llmHorizonPreContract",
        )}
      </div>
    );
  }

  return (
    <div
      data-testid={testId}
      data-state="ok"
      className="flex items-center gap-1.5 flex-wrap text-sm"
      style={{ color: "var(--muted)" }}
    >
      <span>{t("stockAnalysis.llmHorizonOverview")}</span>
      {Object.entries(HORIZON_CAMEL_TO_SNAKE).map(([camel, snake]) => {
        const suffix = HORIZON_T_SUFFIX[snake];
        const row = view.rows[camel as keyof typeof view.rows] as LlmHorizonDecision | undefined;
        return (
          <span
            key={camel}
            data-testid={`llm-horizon-slot-${camel}`}
            data-given={row ? "true" : "false"}
            className="flex items-center gap-1 px-1.5 py-0.5 rounded"
            style={{ background: "var(--surface)", border: "1px solid var(--border)" }}
          >
            <span className="font-medium" style={{ color: "var(--color-text-secondary)" }}>
              {t(`stockAnalysis.timeHorizon${suffix}`)}
            </span>
            {row
              ? (
                <>
                  <Tag color={getActionColor(row.action)} style={{ margin: 0 }}>
                    {t(getActionTKey(row.action))}
                  </Tag>
                  {typeof row.positionPct === "number" && <span className="font-mono">{row.positionPct}%</span>}
                </>
              )
              : <span>{t("stockAnalysis.llmHorizonNotGiven")}</span>}
          </span>
        );
      })}
    </div>
  );
}
