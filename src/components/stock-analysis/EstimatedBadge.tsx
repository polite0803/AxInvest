/**
 * 「估算值」角标组件（A2 审计：前端估值面板展示估算标记）。
 *
 * 消费 `consensusEps` 的估算标记字段：当 `isEstimated === true` 时渲染一个小 Tag 角标，
 * 悬停显示估算来源（`estimateSource`）。`isEstimated` 非 `true`（或字段缺失）时**不渲染任何东西**。
 *
 * 语义对齐后端 `astock-data::types::ConsensusEPS`：`isEstimated = true` 代表该 EPS 是
 * vendor 全失败后的兜底常数估算值，不得用作「超预期 / 不及预期」判定基准。
 */
import type { ConsensusEPS } from "@/types";
import { Tag, Tooltip } from "antd";
import { useTranslation } from "react-i18next";

export interface EstimatedBadgeProps {
  /** 是否为估算值（`ConsensusEPS.isEstimated`） */
  isEstimated?: ConsensusEPS["isEstimated"];
  /** 估算来源，可选项（`ConsensusEPS.estimateSource`，如 "board_constant:star"） */
  estimateSource?: ConsensusEPS["estimateSource"];
}

export function EstimatedBadge({ isEstimated, estimateSource }: EstimatedBadgeProps) {
  const { t } = useTranslation();
  if (isEstimated !== true) {
    return null;
  }
  const badge = (
    <Tag color="orange" style={{ marginInlineEnd: 0 }}>
      {t("stockAnalysis.estimatedBadge")}
    </Tag>
  );
  if (!estimateSource) {
    return badge;
  }
  return (
    <Tooltip title={t("stockAnalysis.estimatedSource", { source: estimateSource })}>
      {badge}
    </Tooltip>
  );
}
