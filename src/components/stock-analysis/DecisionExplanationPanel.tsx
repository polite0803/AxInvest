// SPDX-License-Identifier: AGPL-3.0-only

import { useStockAnalysisStore } from "@/stores";
import { Button, Tag } from "antd";
import { useState } from "react";
import { useTranslation } from "react-i18next";

/**
 * 决策依据说明书 —— 把 `decision-explainer` 节点的产出摊到决策卡上。
 *
 * 存在意义（补上一条「产出零消费端」的断链）：
 * 工作流尾链的 `decision-explainer` 节点会把 `portfolio-mgr`（公式决策）+
 * `portfolio-risk-gate`（组合风控门）的**符号化裁决**（`action=BREAK` /
 * `positionPct=0` / `R-206`）翻译成人话，并给出规则追溯码清单。
 * 但在此之前，该节点的输出**没有任何消费端**：`notify-result` 发的是固定文案，
 * `store-result` / `end-output` 取的都是 `portfolio-risk-gate`，前端也从未读过
 * 它的字段 —— 数据只写进了 `blackboard_snapshot` 无人问津。
 * 用户在决策卡上看到的「依据」实际来自 `decision.reasoning`（一行摘要），
 * 这份更完整的说明书从未露面。
 *
 * 数据来源：`stockAnalysisStore.decisionExplanation`
 *   · 实时路径 — `workflow-completed` 的 `results["decision-explainer"]`
 *   · 回放路径 — `loadAnalysis` 的 `blackboard_snapshot["decision-explainer"]`
 * 两条路径都经 `lib/agentOutput.ts::parseDecisionExplanation` 解包 +
 * 校验（四个内容字段全空则返回 null），因此此处**不需要**再做兜底解析。
 *
 * 无产出时整块不渲染 —— 该节点 `continue_on_fail = true`，失败/跳过是预期状态，
 * 不应该在决策卡上留一条空壳面板。
 */
export function DecisionExplanationPanel() {
  const { t } = useTranslation();
  const explanation = useStockAnalysisStore((s) => s.decisionExplanation);
  const [expanded, setExpanded] = useState(false);

  if (!explanation) { return null; }

  const { summary, explanation: detail, ruleTrace, riskComment, confidenceNote } = explanation;
  const hasDetail = !!(detail || riskComment || confidenceNote);
  const canExpand = hasDetail || ruleTrace.length > 0;

  /** 规则追溯码的执行结果 → 配色。未知值不着色（原样展示，不做猜测性映射）。 */
  const statusColor = (status: string): string => {
    switch (status.toUpperCase()) {
      case "PASS":
        return "green";
      case "VETOED":
        return "red";
      case "DOWNGRADED":
        return "orange";
      default:
        return "default";
    }
  };

  return (
    <div
      className="rounded px-2.5 py-1.5 text-xs"
      style={{
        background: "rgba(22, 119, 255, 0.08)",
        borderLeft: "3px solid var(--sa-blue)",
      }}
    >
      <div className="flex items-center gap-2 flex-wrap">
        <span className="font-semibold" style={{ color: "var(--sa-blue)" }}>
          📘 {t("stockAnalysis.decisionExplanation.title")}
        </span>
        {summary && (
          <span style={{ color: "var(--color-text-primary)" }} title={summary}>
            {expanded || summary.length <= 80 ? summary : `${summary.slice(0, 80)}…`}
          </span>
        )}
        {canExpand && (
          <Button
            type="link"
            size="small"
            style={{ height: "auto", padding: 0, fontSize: 12 }}
            onClick={() => setExpanded((v) => !v)}
          >
            {expanded
              ? t("stockAnalysis.decisionExplanation.collapse")
              : t("stockAnalysis.decisionExplanation.expand")}
          </Button>
        )}
      </div>

      {expanded && (
        <div className="mt-1.5 space-y-1.5">
          {detail && (
            <div style={{ color: "var(--color-text-primary)", whiteSpace: "pre-wrap" }}>
              {detail}
            </div>
          )}

          {ruleTrace.length > 0 && (
            <div>
              <div style={{ color: "var(--muted)" }}>
                {t("stockAnalysis.decisionExplanation.ruleTraceLabel")}
              </div>
              <div className="mt-1 space-y-1">
                {ruleTrace.map((r, i) => (
                  <div key={`${r.ruleId}-${i}`} className="flex items-start gap-1.5">
                    <Tag
                      color={statusColor(r.status)}
                      style={{ margin: 0, fontSize: 11, lineHeight: "18px", paddingInline: 4 }}
                    >
                      {r.ruleId}
                    </Tag>
                    <span style={{ color: "var(--muted)", minWidth: 74 }}>{r.status}</span>
                    <span style={{ color: "var(--color-text-primary)" }}>{r.description}</span>
                  </div>
                ))}
              </div>
            </div>
          )}

          {riskComment && (
            <div>
              <span style={{ color: "var(--muted)" }}>
                {t("stockAnalysis.decisionExplanation.riskCommentLabel")}
                {"："}
              </span>
              <span style={{ color: "var(--color-text-primary)" }}>{riskComment}</span>
            </div>
          )}

          {confidenceNote && (
            <div>
              <span style={{ color: "var(--muted)" }}>
                {t("stockAnalysis.decisionExplanation.confidenceNoteLabel")}
                {"："}
              </span>
              <span style={{ color: "var(--color-text-primary)" }}>{confidenceNote}</span>
            </div>
          )}

          <div style={{ color: "var(--muted)" }}>
            {t("stockAnalysis.decisionExplanation.sourceHint")}
          </div>
        </div>
      )}
    </div>
  );
}
