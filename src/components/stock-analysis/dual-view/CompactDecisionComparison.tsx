// i18n-exempt: 业务逻辑/API 描述/日志字符串，非 UI 展示文本
/**
 * CompactDecisionComparison — 双视角决策对比的 chat bubble 紧凑版
 *
 * 方案 D 双向并存:展示"LLM 视角 vs 公式视角"对比的紧凑 2-3 行视图。
 * 用于 chat bubble 嵌入,默认 `<DecisionComparisonPanel />` 完整版用 `<DualViewRenderer>` 切换。
 *
 * 数据契约:与后端 `stock_analyses` 表的字段对应,前端 `AnalysisSummary` 接口已扩展。
 *
 * 显示规则:
 * - LLM 决策可用时:两行(公式 / LLM 各一行),底部一致性分数
 * - LLM 不可用时:一行占位 + 灰条"LLM 视角不可用"
 */
import {
  actionToDirection,
  agreementBgColor,
  agreementColor,
  getActionTKey,
  parseAction,
  resolveDisplayAction,
} from "@/lib/stock-analysis-utils";
import { useMemo } from "react";
import { useTranslation } from "react-i18next";

/** 与 AnalysisSummary / LatestAnalysisSummary 的 LLM 字段子集对齐 */
export interface CompactDecisionShape {
  decisionAction?: string | null;
  /** 持仓状态轴（v228）；null = 记录早于 v228，非 EMPTY */
  decisionPositionState?: string | null;
  decisionPositionPct?: number | null;
  confidence?: number | null;
  /** V50: 双视角一致性调制后的置信度 */
  adjustedConfidence?: number | null;
  /** 公式决策推理文本（用于展示公式 reasoning） */
  decisionReasoning?: string | null;
  llmDecisionAction?: string | null;
  llmDecisionPositionPct?: number | null;
  llmConfidence?: number | null;
  /** V65: LLM 风险等级 */
  llmRiskLevel?: string | null;
  /** V65: LLM 止损百分比 */
  llmStopLossPct?: number | null;
  /** V65: LLM 止盈百分比 */
  llmTakeProfitPct?: number | null;
  /** V65: LLM 数据缺口列表 */
  llmDataGaps?: string[] | null;
  /** V65: LLM 引用上游论据 */
  llmEvidenceCited?: Array<{ source?: string; point?: string }> | null;
  /** LLM 决策推理文本(用于 panel 完整版展示) */
  llmDecisionReasoning?: string | null;
  decisionAgreementScore?: number | null;
  /** V65: 双视角一致性 6 维度诊断 */
  agreementBreakdown?: {
    total: number;
    actionOk: boolean;
    actionNote: string;
    formulaAction: string;
    llmAction: string;
    actionScore?: number;
    positionScore?: number;
    positionGap: number | null;
    confidenceScore?: number;
    confidenceGap: number | null;
    riskLevelScore?: number;
    formulaRiskLevel?: string;
    llmRiskLevel?: string;
    // 2026-09-21 移除 dataGapsScore / dataGapsSimilarity（随 data_gaps 一致性维度删除）
    evidenceScore?: number;
    evidenceCount?: number;
    conflictType: string;
  } | null;
}

interface CompactDecisionComparisonProps {
  data: CompactDecisionShape | unknown;
}

function normalize(data: CompactDecisionComparisonProps["data"]): CompactDecisionShape {
  if (data && typeof data === "object") {
    return data as CompactDecisionShape;
  }
  return {};
}

/**
 * 把 action 字符串映射成"行动风格"色标（买 → 蓝绿；卖 → 红；中性/未知 → 灰）。
 *
 * P1-6(2026-09-14): 原判据用 `norm.includes("买")` —— **只认中文**，英文 token
 * （`BUY`/`SELL`，后端 DTO 明确可能直返）一律落到灰色，双视角对比里换值域即掉配色。
 * 改走统一的 `actionToDirection`（已覆盖中英文两套值域）。
 */
function actionColor(action?: string | null): string {
  const dir = actionToDirection(action);
  if (dir === "buy") { return "#10b981"; }
  if (dir === "sell") { return "#ef4444"; }
  return "#94a3b8";
}

export function CompactDecisionComparison({ data }: CompactDecisionComparisonProps) {
  const { t } = useTranslation();
  const view = useMemo(() => normalize(data), [data]);

  const hasLlm = view.llmDecisionAction != null
    && view.llmDecisionAction !== ""
    && view.llmDecisionAction !== "null";
  const agreement = typeof view.decisionAgreementScore === "number" ? view.decisionAgreementScore : null;
  const actionsMatch = hasLlm
    // P1-6(2026-09-14): 原按字符串 trim/lower 后直接比字面量 ⇒ 「公式=BUY vs LLM=买入」
    // 被判成「不一致」。改走 parseAction 统一值域后比较（比语义，不比字形）。
    && parseAction(view.decisionAction) === parseAction(view.llmDecisionAction);

  return (
    <div className="space-y-1.5 text-sm">
      {/* 一致性色条 + 分数 */}
      <div className="flex items-center gap-2">
        <span style={{ color: "var(--muted)" }}>{t("dualView.decision.title")}</span>
        {agreement !== null && (
          <div
            className="flex items-center gap-1 px-1.5 rounded text-sm font-mono"
            style={{
              // 2026-09-21: 阈值与配色统一走 `agreementBgColor` / `agreementColor`
              // （单一真相源），不再在此内联一份 60/40 档位表。
              background: agreementBgColor(agreement),
              color: agreementColor(agreement),
            }}
          >
            <span className="font-semibold">{agreement}</span>
            <span style={{ opacity: 0.7 }}>/100</span>
          </div>
        )}
      </div>

      {/* 公式行 */}
      <div className="flex items-center gap-1.5">
        <span
          className="px-1 rounded text-sm font-medium"
          style={{ background: "var(--sa-blue-bg, #dbeafe)", color: "#2563eb" }}
        >
          {t("dualView.decision.formulaBadge")}
        </span>
        <span
          className="font-mono text-sm font-semibold"
          style={{ color: actionColor(view.decisionAction) }}
        >
          {view.decisionAction
            ? t(
              getActionTKey(
                resolveDisplayAction(
                  view.decisionAction,
                  view.decisionPositionState,
                  view.decisionPositionPct,
                ),
              ),
            )
            : "—"}
        </span>
        {typeof view.decisionPositionPct === "number" && (
          <span className="text-sm font-mono" style={{ color: "var(--muted)" }}>
            {view.decisionPositionPct.toFixed(0)}%
          </span>
        )}
      </div>

      {/* LLM 行(不可用时显示灰条) */}
      {hasLlm
        ? (
          <div className="flex items-center gap-1.5">
            <span
              className="px-1 rounded text-sm font-medium"
              style={{ background: "var(--sa-purple-bg, #ede9fe)", color: "#7c3aed" }}
            >
              {t("dualView.decision.llmBadge")}
            </span>
            <span
              className="font-mono text-sm font-semibold"
              style={{ color: actionColor(view.llmDecisionAction) }}
            >
              {t(getActionTKey(view.llmDecisionAction ?? ""))}
            </span>
            {typeof view.llmDecisionPositionPct === "number" && (
              <span className="text-sm font-mono" style={{ color: "var(--muted)" }}>
                {view.llmDecisionPositionPct.toFixed(0)}%
              </span>
            )}
            {actionsMatch && (
              <span
                className="text-sm px-1 rounded"
                style={{ background: "rgba(16, 185, 129, 0.15)", color: "#10b981" }}
              >
                ✓
              </span>
            )}
          </div>
        )
        : (
          <div
            className="flex items-center gap-1.5 italic"
            style={{ color: "var(--muted)" }}
          >
            <span
              className="px-1 rounded text-sm font-medium"
              style={{ background: "var(--sa-purple-bg, #ede9fe)", color: "#7c3aed" }}
            >
              {t("dualView.decision.llmBadge")}
            </span>
            <span className="text-sm">{t("dualView.decision.llmUnavailable")}</span>
          </div>
        )}
      {/* V50: 分歧诊断摘要（仅低一致时显示） */}
      {agreement !== null && agreement < 60 && view.agreementBreakdown && (
        <div className="text-sm" style={{ color: "#ef4444", opacity: 0.85 }}>
          {view.agreementBreakdown.actionNote === "opposite"
            ? `⚠ ${view.agreementBreakdown.formulaAction} ≠ ${view.agreementBreakdown.llmAction}（${
              t("stockAnalysis.decision.oppositeDirection")
            }）`
            : `⚠ ${t("stockAnalysis.decision.disagreement")}（${
              view.agreementBreakdown.conflictType === "opposite_direction"
                ? t("stockAnalysis.decision.directionConflict")
                : view.agreementBreakdown.conflictType
            }）`}
        </div>
      )}
    </div>
  );
}
