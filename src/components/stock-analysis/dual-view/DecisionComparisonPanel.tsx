// i18n-exempt: 业务逻辑/API 描述/日志字符串，非 UI 展示文本
/**
 * DecisionComparisonPanel — 双视角决策对比的紧凑面板版
 *
 * 紧凑设计原则：
 *   - 标题 + 一致性分数同行（色条+数字，无圆环）
 *   - 公式/LLM 双列并排卡片，每列内: action tag + conf% | pos% 单行
 *   - reasoning 展开前 line-clamp-2，点击展开全文
 *   - 分歧诊断用 Tooltip 内联（不另占卡片空间）
 */
import { actionToDirection, getActionTKey, parseAction, resolveDisplayAction } from "@/lib/stock-analysis-utils";
import { Empty, Tag, Tooltip } from "antd";
import { useMemo, useState } from "react";
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

/**
 * 展示档派生（P1-2 2026-09-14）。
 *
 * 「持有 / 观望」不再由组件各自判断仓位：后端目前仍按仓位互改 action，
 * `resolveDisplayAction` 与后端同判据（positionState 优先，缺失时退回 positionPct），
 * 故对既有数据是恒等变换；后端切到「action 只表达方向强度」后此处无需再改。
 *
 * V76(2026-09-14): 后端 DTO 已下发 `decisionPositionState`（v228 独立轴），
 * 本函数优先用它；为 `null`（v228 前的历史行）时才退回 `decisionPositionPct`。
 */
function displayActionOf(
  view: {
    decisionAction?: string | null;
    decisionPositionState?: string | null;
    decisionPositionPct?: number | null;
  },
): string {
  return resolveDisplayAction(
    view.decisionAction,
    view.decisionPositionState,
    view.decisionPositionPct,
  );
}

// P1-6(2026-09-14): 原判据用 `norm.includes("买")` —— **只认中文**。
//   后端 DTO 文档写明 decisionAction 可能直返 `BUY/SELL/HOLD/WAIT/UNCERTAIN`（英文 token），
//   这一类输入会全部落到 `default`（灰）⇒ 双视角对比里「公式」侧永远没有红/绿配色。
//   改走统一的 `actionToDirection`（已覆盖中英文两套值域）。
function actionTagColor(action?: string | null): string {
  const dir = actionToDirection(action);
  if (dir === "buy") { return "green"; }
  if (dir === "sell") { return "red"; }
  return "default";
}

function agreementLevel(score: number): "high" | "mid" | "low" {
  if (score >= 80) { return "high"; }
  if (score >= 40) { return "mid"; }
  return "low";
}

/** 带缺失提示的数值组件 */
function NumValue({ value, suffix = "" }: { value: number | null | undefined; suffix?: string }) {
  const { t } = useTranslation();
  if (typeof value !== "number" || !Number.isFinite(value)) {
    return <span title={t("common.noData")}>—</span>;
  }
  return <>{value.toFixed(0)}{suffix}</>;
}

/** 检查两列 action 是否不一致 */
/**
 * 双视角「动作是否分歧」判据。
 *
 * P1-6(2026-09-14): 原实现把两侧字符串各自 trim/lower/去分隔符后**直接比字面量**，
 * 于是「公式侧 = `BUY`（英文 token） vs LLM 侧 = `买入`（中文）」被判成**分歧** ——
 * 同义不同值域制造了假的分歧告警（与本案 P0-2「从自由文本猜方向」同一类：
 * 比的是字形而不是语义）。现走 `parseAction` 统一到同一值域再比。
 *
 * ⚠️ 刻意**不**用 `resolveDisplayAction`：那是把「持有 / 观望」按持仓派生的**展示**档，
 *    用来比「方向是否分歧」会掩盖两侧在持仓语义上的真实差异。
 */
function actionsDiffer(a?: string | null, b?: string | null): boolean {
  if (!a || !b) { return false; }
  return parseAction(a) !== parseAction(b);
}

export function DecisionComparisonPanel({ data }: DecisionComparisonPanelProps) {
  const { t } = useTranslation();
  const [expandedReasoning, setExpandedReasoning] = useState(false);
  const view = useMemo(() => normalize(data), [data]);

  const hasLlm = view.llmDecisionAction != null
    && view.llmDecisionAction !== ""
    && view.llmDecisionAction !== "null";
  const agreement = typeof view.decisionAgreementScore === "number" ? view.decisionAgreementScore : null;

  // LLM 不可用 → 紧凑回退
  if (!hasLlm && agreement === null) {
    const hasFormula = view.decisionAction != null && view.decisionAction !== "" && view.decisionAction !== "null";
    if (!hasFormula) {
      return (
        <div className="decision-comparison-panel p-2">
          <Empty image={Empty.PRESENTED_IMAGE_SIMPLE} description={t("dualView.decision.llmUnavailableHint")} />
        </div>
      );
    }
    return (
      <div className="decision-comparison-panel p-2 space-y-1 text-sm">
        <Tag color="warning">{t("stockAnalysis.llmViewUnavailable")}</Tag>
        <div className="grid grid-cols-2 gap-x-3 gap-y-0.5">
          <span style={{ color: "var(--muted)" }}>{t("dualView.decision.action")}</span>
          <span>
            <Tag color={actionTagColor(view.decisionAction)}>
              {t(getActionTKey(displayActionOf(view)))}
            </Tag>
          </span>
          <span style={{ color: "var(--muted)" }}>{t("dualView.decision.confidence")}</span>
          <span className="font-mono">
            <NumValue value={view.confidence} />
          </span>
        </div>
      </div>
    );
  }

  const level = agreement !== null ? agreementLevel(agreement) : null;
  const agreeColor = level === "high" ? "#10b981" : level === "mid" ? "#f59e0b" : "#ef4444";
  const agreeBg = level === "high"
    ? "rgba(16,185,129,0.10)"
    : level === "mid"
    ? "rgba(245,158,11,0.10)"
    : "rgba(239,68,68,0.10)";

  const formulaReasoning = view.decisionReasoning || "";
  const llmReasoning = view.llmDecisionReasoning || "";
  const actionDiverged = hasLlm && actionsDiffer(view.decisionAction, view.llmDecisionAction);
  const divBorderColor = actionDiverged ? "rgba(239,68,68,0.30)" : undefined;

  return (
    <div className="decision-comparison-panel p-2 space-y-1.5 text-sm">
      {/* ═══ 第1行：标题 + 一致性色条 + 分歧诊断(Tooltip) + 展开按钮 ═══ */}
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-1.5">
          <span className="text-sm font-semibold">{t("dualView.decision.title")}</span>
          {agreement !== null && (
            <span
              className="flex items-center gap-0.5 px-1.5 py-0.5 rounded-full text-sm font-mono font-medium"
              style={{ background: agreeBg, color: agreeColor }}
            >
              {agreement}
              <span className="opacity-60">/100</span>
            </span>
          )}
          {/* 分歧诊断 — 内联 Tooltip，悬停查看详情 */}
          {view.agreementBreakdown && agreement !== null && agreement < 80 && (
            <Tooltip
              title={
                <div className="text-sm space-y-0.5">
                  <div>
                    {t("dualView.decision.action")}: {view.agreementBreakdown.actionNote === "opposite"
                      ? t("stockAnalysis.decision.opposite")
                      : view.agreementBreakdown.actionNote === "same_direction"
                      ? t("stockAnalysis.decision.sameDirection")
                      : t("stockAnalysis.decision.disagreement")} ({view.agreementBreakdown.formulaAction} vs{" "}
                    {view.agreementBreakdown.llmAction})
                  </div>
                  {view.agreementBreakdown.positionGap != null && (
                    <div>
                      {t("dualView.decision.positionPct")}: {t("stockAnalysis.decision.diff")}{" "}
                      {Math.round(view.agreementBreakdown.positionGap)}%
                    </div>
                  )}
                  {view.agreementBreakdown.confidenceGap != null && (
                    <div>
                      {t("dualView.decision.confidence")}: {t("stockAnalysis.decision.diff")}{" "}
                      {Math.round(view.agreementBreakdown.confidenceGap)}%
                    </div>
                  )}
                </div>
              }
            >
              <span
                className="cursor-help px-1 py-0.5 rounded text-sm underline decoration-dotted"
                style={{ color: "#ef4444", opacity: 0.85 }}
              >
                ⚠{view.agreementBreakdown.actionNote === "opposite"
                  ? ` ${t("dualView.decision.reviewRecommended")}`
                  : ""}
              </span>
            </Tooltip>
          )}
        </div>
        {/* reasoning 超长时显示展开按钮 */}
        {!expandedReasoning && (formulaReasoning.length > 80 || llmReasoning.length > 80) && (
          <button
            className="text-sm hover:opacity-70 transition-opacity"
            style={{
              color: "var(--accent, #6366f1)",
              border: "none",
              background: "none",
              padding: 0,
              cursor: "pointer",
            }}
            onClick={() => setExpandedReasoning(true)}
          >
            {t("stockAnalysis.showDetail")} ▸
          </button>
        )}
      </div>

      {/* ═══ 第2行：双列对比卡片（公式 | LLM）═══ */}
      <div className="grid grid-cols-2 gap-2">
        {/* ── 公式列 ── */}
        <div
          className="rounded-md p-1.5 space-y-1"
          style={{
            background: "rgba(37,99,235,0.05)",
            border: `1px solid ${divBorderColor ?? "rgba(37,99,235,0.10)"}`,
          }}
        >
          <div className="flex items-center justify-between">
            <span
              className="text-sm font-medium px-1 py-px rounded"
              style={{ background: "rgba(37,99,235,0.10)", color: "#2563eb" }}
            >
              {t("dualView.decision.formula")}
            </span>
            {view.decisionAction
              ? (
                <Tag color={actionTagColor(view.decisionAction)}>
                  {t(getActionTKey(displayActionOf(view)))}
                </Tag>
              )
              : <span style={{ color: "var(--muted)" }}>—</span>}
          </div>
          <div className="flex items-center gap-1.5 font-mono text-sm">
            <span style={{ color: "var(--color-text-secondary)" }}>
              {t("stockAnalysis.decision.confidenceLabel")}{" "}
              <b style={{ fontSize: "13px" }}>
                <NumValue value={view.confidence} suffix="%" />
              </b>
            </span>
            <span style={{ color: "var(--muted)", opacity: 0.4 }}>|</span>
            <span style={{ color: "var(--color-text-secondary)" }}>
              {t("stockAnalysis.decision.positionLabel")}{" "}
              <b style={{ fontSize: "13px" }}>
                <NumValue value={view.decisionPositionPct} suffix="%" />
              </b>
            </span>
          </div>
          {formulaReasoning && (
            <div
              className={`leading-tight cursor-pointer hover:opacity-70 transition-opacity ${
                expandedReasoning ? "" : "line-clamp-2"
              }`}
              style={{ color: "var(--color-text-secondary)", fontSize: "12px" }}
              onClick={() => setExpandedReasoning(!expandedReasoning)}
              title={expandedReasoning ? t("stockAnalysis.collapseComparison") : formulaReasoning}
            >
              {formulaReasoning}
            </div>
          )}
        </div>

        {/* ── LLM 列 ── */}
        <div
          className="rounded-md p-1.5 space-y-1"
          style={{
            background: "rgba(124,58,237,0.05)",
            border: `1px solid ${divBorderColor ?? "rgba(124,58,237,0.10)"}`,
          }}
        >
          <div className="flex items-center justify-between">
            <span
              className="text-sm font-medium px-1 py-px rounded"
              style={{ background: "rgba(124,58,237,0.10)", color: "#7c3aed" }}
            >
              {t("dualView.decision.llm")}
            </span>
            {hasLlm
              ? (
                <Tag color={actionTagColor(view.llmDecisionAction)}>
                  {t(getActionTKey(view.llmDecisionAction ?? ""))}
                </Tag>
              )
              : <span style={{ color: "var(--muted)" }}>—</span>}
          </div>
          <div className="flex items-center gap-1.5 font-mono text-sm">
            <span style={{ color: "var(--color-text-secondary)" }}>
              {t("stockAnalysis.decision.confidenceLabel")}{" "}
              <b style={{ fontSize: "13px" }}>
                <NumValue value={view.llmConfidence} suffix="%" />
              </b>
            </span>
            <span style={{ color: "var(--muted)", opacity: 0.4 }}>|</span>
            <span style={{ color: "var(--color-text-secondary)" }}>
              {t("stockAnalysis.decision.positionLabel")}{" "}
              <b style={{ fontSize: "13px" }}>
                <NumValue value={view.llmDecisionPositionPct} suffix="%" />
              </b>
            </span>
          </div>
          {llmReasoning && (
            <div
              className={`leading-tight cursor-pointer hover:opacity-70 transition-opacity ${
                expandedReasoning ? "" : "line-clamp-2"
              }`}
              style={{ color: "var(--color-text-secondary)", fontSize: "12px" }}
              onClick={() => setExpandedReasoning(!expandedReasoning)}
              title={expandedReasoning ? t("stockAnalysis.collapseComparison") : llmReasoning}
            >
              {llmReasoning}
            </div>
          )}
        </div>
      </div>

      {/* LLM 缺失提示 */}
      {!hasLlm && agreement !== null && (
        <div
          className="text-sm italic px-1.5 py-0.5 rounded"
          style={{ background: "var(--surface)", color: "var(--muted)" }}
        >
          {t("dualView.decision.llmMissingHint", { score: agreement })}
        </div>
      )}

      {/* 收起按钮 */}
      {expandedReasoning && (
        <button
          className="text-sm hover:opacity-70 transition-opacity"
          style={{ color: "var(--muted)", border: "none", background: "none", padding: 0, cursor: "pointer" }}
          onClick={() => setExpandedReasoning(false)}
        >
          ▾ {t("stockAnalysis.collapseComparison")}
        </button>
      )}
    </div>
  );
}
