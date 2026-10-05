import type { TFunction } from "i18next";

/**
 * 工作流节点 ID → 用户可见标签
 * - 优先尝试 i18n key：`stockAnalysis.workflow.${nodeId}`
 * - 特殊节点（翻译文件 key 与 ID 不完全对齐）走显式映射
 * - 全部 miss 时返回 nodeId 本身
 */
const SPECIAL_MAP: Record<string, string> = {
  "cls-risk-level": "stockAnalysis.workflow.riskLevel",
  "risk-level": "stockAnalysis.workflow.riskLevel",
  "agg-risk": "stockAnalysis.workflow.aggRisk",
  "risk-agg": "stockAnalysis.workflow.riskAggregation",
  "risk-con": "stockAnalysis.workflow.riskConservative",
  "risk-neu": "stockAnalysis.workflow.riskNeutral",
  "risk-aggregated": "stockAnalysis.workflow.riskAggregation",
  "risk-convergence": "stockAnalysis.workflow.riskConvergence",
  "debate-convergence": "stockAnalysis.analysisDebug.debateConvergence",
  "v-validate": "stockAnalysis.workflow.vValidate",
  "notify-result": "stockAnalysis.workflow.notifyResult",
};

/**
 * v128（B1）四个逐档风险节点的档位后缀 → 档位名 i18n key。
 * 组合而非新建 key：`风险等级分类 · 中期` —— 四档在失败/状态行里必须分得清是哪一档，
 * 否则「按档」在呈现层又退化成四个同名条目（用户裁定：UI 相关位置要有四周期输出）。
 */
const HORIZON_RISK_NODE_PREFIX = "cls-risk-level-";
const TIER_SUFFIX_TO_KEY: Record<string, string> = {
  "ultra-short": "stockAnalysis.timeHorizonUltraShort",
  "short": "stockAnalysis.timeHorizonShort",
  "mid": "stockAnalysis.timeHorizonMid",
  "long": "stockAnalysis.timeHorizonLong",
};

export function getWorkflowNodeLabel(nodeId: string, t: TFunction): string {
  if (nodeId.startsWith(HORIZON_RISK_NODE_PREFIX)) {
    const suffix = nodeId.slice(HORIZON_RISK_NODE_PREFIX.length);
    const horizonKey = TIER_SUFFIX_TO_KEY[suffix];
    if (horizonKey) {
      return `${t("stockAnalysis.workflow.riskLevel")} · ${t(horizonKey)}`;
    }
  }
  const key = SPECIAL_MAP[nodeId] ?? `stockAnalysis.workflow.${nodeId}`;
  const result = t(key, { defaultValue: nodeId });
  // Some keys (e.g. "analyst", "phase") resolve to i18n objects, not strings
  return typeof result === "string" ? result : nodeId;
}
