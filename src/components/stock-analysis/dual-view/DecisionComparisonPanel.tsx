/**
 * DecisionComparisonPanel — 双视角决策对比的完整面板版
 *
 * 方案 D 双向并存:展示"LLM 视角 vs 公式视角"完整对比。
 * 挂在 `<DualViewRenderer id="decision-comparison" />` 出口。
 *
 * 字段对比:
 *   - action(必显示,差异高亮)
 *   - positionPct(差异 ≤ 5pct 视为一致)
 *   - confidence(差异 ≤ 10 视为一致)
 *   - timeHorizon
 *   - reasoning(完整推理文本对比)
 *
 * 一致性:
 *   - 顶部 0-100 圆环 + 解读文案
 *   - ≥ 80 高一致(绿) / 40-79 中(黄) / < 40 分歧(红)
 *   - < 40 时提示"建议人工复核"
 */
import { Empty, Progress, Tag } from "antd";
import { useMemo } from "react";
import { useTranslation } from "react-i18next";
import type { CompactDecisionShape } from "./CompactDecisionComparison";

interface DecisionComparisonPanelProps {
  data: CompactDecisionShape | unknown;
}

function normalize(data: DecisionComparisonPanelProps["data"]): CompactDecisionShape {
  if (data && typeof data === "object") {
    return data as CompactDecisionShape;
  }
  return {};
}

function normalizeAction(a?: string | null): string {
  return (a ?? "").trim().toLowerCase().replace(/[\s/_\u3000]+/g, "");
}

function actionTagColor(action?: string | null): string {
  const norm = normalizeAction(action);
  if (norm.includes("买") || norm.includes("增持")) { return "green"; }
  if (norm.includes("卖") || norm.includes("减持")) { return "red"; }
  return "default";
}

function agreementLevel(score: number): "high" | "mid" | "low" {
  if (score >= 80) { return "high"; }
  if (score >= 40) { return "mid"; }
  return "low";
}

function progressStrokeColor(score: number): string {
  const level = agreementLevel(score);
  if (level === "high") { return "#10b981"; }
  if (level === "mid") { return "#f59e0b"; }
  return "#ef4444";
}

/** 数字格式化:undefined/null → "—", 否则保留 0 位小数 */
function fmtNum(v: number | null | undefined, suffix = ""): string {
  if (typeof v !== "number" || !Number.isFinite(v)) { return "—"; }
  return `${v.toFixed(0)}${suffix}`;
}

export function DecisionComparisonPanel({ data }: DecisionComparisonPanelProps) {
  const { t } = useTranslation();
  const view = useMemo(() => normalize(data), [data]);

  const hasLlm = view.llmDecisionAction != null
    && view.llmDecisionAction !== ""
    && view.llmDecisionAction !== "null";
  const agreement = typeof view.decisionAgreementScore === "number" ? view.decisionAgreementScore : null;
  const actionsMatch = hasLlm
    && normalizeAction(view.decisionAction) === normalizeAction(view.llmDecisionAction);
  const positionMatch = typeof view.decisionPositionPct === "number"
    && typeof view.llmDecisionPositionPct === "number"
    && Math.abs(view.decisionPositionPct - view.llmDecisionPositionPct) <= 5;

  // LLM 视角完全不可用 → 回退显示公式决策单视角
  if (!hasLlm && agreement === null) {
    // 即使公式决策也存在不足时显示空状态
    const hasFormula = view.decisionAction != null && view.decisionAction !== "" && view.decisionAction !== "null";
    if (!hasFormula) {
      return (
        <div className="decision-comparison-panel p-4">
          <Empty
            image={Empty.PRESENTED_IMAGE_SIMPLE}
            description={t("dualView.decision.llmUnavailableHint")}
          />
        </div>
      );
    }
    // 有公式决策但无 LLM 双视角 → 降级显示公式决策单列
    return (
      <div className="decision-comparison-panel p-3 space-y-3">
        <div className="flex items-center gap-2 mb-2">
          <Tag color="warning">{t("stockAnalysis.llmViewUnavailable")}</Tag>
          <span className="text-xs" style={{ color: "var(--muted)" }}>
            {t("stockAnalysis.formulaFallbackHint")}
          </span>
        </div>
        <div className="grid grid-cols-2 gap-2 text-[12px]">
          <div className="font-semibold" style={{ color: "var(--muted)" }}>{t("dualView.decision.field")}</div>
          <div className="font-semibold text-center" style={{ color: "#2563eb" }}>
            {t("dualView.decision.formula")}
          </div>
          <div>{t("dualView.decision.action")}</div>
          <div className="text-center">
            <Tag color={actionTagColor(view.decisionAction)}>{view.decisionAction}</Tag>
          </div>
          <div>{t("dualView.decision.positionPct")}</div>
          <div className="text-center font-mono">{fmtNum(view.decisionPositionPct, "%")}</div>
          <div>{t("dualView.decision.confidence")}</div>
          <div className="text-center font-mono">{fmtNum(view.confidence, "")}</div>
          <div>{t("dualView.decision.reasoning")}</div>
          <div className="text-[11px] whitespace-pre-wrap">{view.llmDecisionReasoning || view.decisionAction}</div>
        </div>
      </div>
    );
  }

  const level = agreement !== null ? agreementLevel(agreement) : null;
  const hintKey = level === "high"
    ? "highAgreementHint"
    : level === "low"
    ? "lowAgreementHint"
    : "midAgreementHint";

  return (
    <div className="decision-comparison-panel p-3 space-y-3">
      {/* 顶部:标题 + 一致性分数 */}
      <div className="flex items-center justify-between gap-4">
        <div className="text-sm font-semibold">{t("dualView.decision.title")}</div>
        {agreement !== null && (
          <div className="flex items-center gap-2">
            <Progress
              type="circle"
              percent={agreement}
              size={48}
              strokeColor={progressStrokeColor(agreement)}
              format={(p) => <span className="text-[12px] font-mono">{p}</span>}
            />
            <div className="text-[11px]" style={{ color: "var(--muted)" }}>
              <div>{t(`dualView.decision.${hintKey}`)}</div>
              {level === "low" && (
                <div className="text-[10px] mt-0.5" style={{ color: "#ef4444" }}>
                  {t("dualView.decision.reviewRecommended")}
                </div>
              )}
            </div>
          </div>
        )}
      </div>

      {/* 对比表格 */}
      <div className="grid grid-cols-3 gap-2 text-[12px]">
        <div className="font-semibold" style={{ color: "var(--muted)" }}>{t("dualView.decision.field")}</div>
        <div className="font-semibold text-center" style={{ color: "#2563eb" }}>
          {t("dualView.decision.formula")}
        </div>
        <div className="font-semibold text-center" style={{ color: "#7c3aed" }}>
          {t("dualView.decision.llm")}
        </div>

        {/* action */}
        <div>{t("dualView.decision.action")}</div>
        <div className="text-center">
          {view.decisionAction
            ? <Tag color={actionTagColor(view.decisionAction)}>{view.decisionAction}</Tag>
            : <span style={{ color: "var(--muted)" }}>—</span>}
        </div>
        <div className="text-center">
          {hasLlm
            ? (
              <Tag
                color={actionTagColor(view.llmDecisionAction)}
                style={actionsMatch ? { borderColor: "#10b981" } : undefined}
              >
                {view.llmDecisionAction}
              </Tag>
            )
            : <span style={{ color: "var(--muted)" }}>—</span>}
        </div>

        {/* positionPct */}
        <div>{t("dualView.decision.positionPct")}</div>
        <div className="text-center font-mono">{fmtNum(view.decisionPositionPct, "%")}</div>
        <div
          className="text-center font-mono"
          style={positionMatch ? { color: "#10b981" } : undefined}
        >
          {fmtNum(view.llmDecisionPositionPct, "%")}
        </div>

        {/* confidence */}
        <div>{t("dualView.decision.confidence")}</div>
        <div className="text-center font-mono">{fmtNum(view.confidence)}</div>
        <div className="text-center font-mono">{fmtNum(view.llmConfidence)}</div>

        {/* reasoning(只在两边都非空时显示) */}
        {view.llmDecisionReasoning && (
          <>
            <div className="pt-1">{t("dualView.decision.reasoning")}</div>
            <div
              className="pt-1 text-[11px]"
              style={{ color: "var(--color-text-secondary)" }}
            >
              {t("dualView.decision.formulaReasoningOmitted")}
            </div>
            <div className="pt-1 text-[11px]" style={{ color: "var(--color-text-secondary)" }}>
              {view.llmDecisionReasoning}
            </div>
          </>
        )}
      </div>

      {/* LLM 不可用但 agreement 存在:解释来源 */}
      {!hasLlm && agreement !== null && (
        <div
          className="text-[11px] italic px-2 py-1 rounded"
          style={{ background: "var(--bg-soft, #f5f5f5)", color: "var(--muted)" }}
        >
          {t("dualView.decision.llmMissingHint", { score: agreement })}
        </div>
      )}
    </div>
  );
}
