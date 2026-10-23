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
  agreementBgColor,
  agreementColor,
  getActionColor,
  getActionTKey,
  isAbsentDecisionAction,
  parseAction,
  resolveDisplayAction,
} from "@/lib/stock-analysis-utils";
import type { DecisionsByHorizon, HorizonAgreement, HorizonDivergence } from "@/types";
import { useMemo } from "react";
import { useTranslation } from "react-i18next";
import { HorizonDecisionStrip } from "../HorizonDecisionStrip";

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
  /**
   * 阶段2：四周期独立决策（真实数据，不伪造）。决策 Tab 与 chat 气泡都按档并列展示；
   * 缺席（本字段引入前的记录）时为 null/undefined ⇒ `HorizonDecisionStrip` 整条不渲染。
   */
  decisionsByHorizon?: DecisionsByHorizon | null;
  /**
   * B1：逐档「公式 vs LLM」对照四行。键缺席 = 该记录产自逐档对照上线前
   * （由 `HorizonAgreementList` 说成「未产出」，不渲染成四个 0 分）。
   * ⚠ 与主档一致性分数不同计数域：逐档三维 / 满分 65，主档五维 / 满分 90。
   */
  agreementByHorizon?: HorizonAgreement[] | null;
  /** B2：逐档分歧归因（渲染随 i18n 码表那一批一起上，见 PLAN §19） */
  divergenceByHorizon?: HorizonDivergence[] | null;
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
 * 档位配色**只有权威一张表**（`ACTION_PALETTE`）。
 *
 * 2026-10-22 收编：本文件旧表按「买 → `#10b981` 绿 / 卖 → `#ef4444` 红」上色，
 * 方向与 A 股涨跌习惯（红=利好）**相反** ⇒ 同一条「买入」在历史弹窗里是红的、
 * 在这张紧凑对比卡里是绿的 —— 那不是「两种风格」，是同一结论两个颜色互相打脸。
 * ⚠ 有意的行为变化：本卡的买入/增持由绿转红、卖出/减持由红转绿。
 */
function actionColor(action?: string | null): string {
  return action ? getActionColor(action) : "var(--muted)";
}

export function CompactDecisionComparison({ data }: CompactDecisionComparisonProps) {
  const { t } = useTranslation();
  const view = useMemo(() => normalize(data), [data]);

  // 「有没有结论」走共用判据（含权威缺失哨兵），不再手写三段字面量比较
  const hasLlm = !isAbsentDecisionAction(view.llmDecisionAction);
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
      {/* 阶段2：四周期逐档决策（有数据才渲染，缺席不补格） */}
      <HorizonDecisionStrip
        decisions={view.decisionsByHorizon}
        mode="decision"
        testId="compact-decision-horizon-strip"
      />
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
