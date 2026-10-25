import { contradictionBasesForTier } from "@/lib/dataQualityDiagnosis";
import {
  type AnalystSentimentTally,
  type Consensus,
  deriveAnalystConsensus,
  groupAnalystReportsByTier,
  HORIZON_T_SUFFIX,
  tallyAnalystSentiment,
} from "@/lib/stock-analysis-utils";
import type { DataQualityReport } from "@/types";
import { useMemo, useState } from "react";
import { cleanToolCallTags } from "./utils";

export interface TierConsensus {
  tier: string;
  tally: AnalystSentimentTally;
  /**
   * 本档被扣除的票数（「结论方向与多空评分互相打脸」的分析师）。
   * 判据在后端，`data-quality` 没给这个字段时恒为 0 —— **不知道**不等于**没人矛盾**，
   * 但计票口径必须保持「按原样计」，所以这里只是不加扣；呈现层据 `excluded > 0` 说明原因。
   */
  excluded: number;
}

/** 档位显示序 = `HORIZON_T_SUFFIX` 的键序（超短 → 短 → 中 → 长）；认不出的档排最后。 */
const TIER_ORDER = Object.keys(HORIZON_T_SUFFIX);
const tierRank = (tier: string): number => {
  const i = TIER_ORDER.indexOf(tier);
  return i < 0 ? TIER_ORDER.length : i;
};

/**
 * 按「结论方向与多空评分矛盾」扣除后计票。
 * `excluded === null`（后端没给这个字段 = **不知道**）时原样计票，绝不把「不知道」
 * 当成「无人矛盾」而扣分，也不当成「全都矛盾」而清空。
 */
function countTally(
  byBase: Record<string, string>,
  excluded: Set<string> | null,
): { tally: AnalystSentimentTally; excluded: number } {
  const entries = Object.entries(byBase);
  const counted = excluded ? entries.filter(([base]) => !excluded.has(base)) : entries;
  return {
    tally: tallyAnalystSentiment(counted.map(([, text]) => text)),
    excluded: entries.length - counted.length,
  };
}

/**
 * 分析师的**逐档共识**（每档自己的票数与结论）。
 *
 * 为什么不是一个合计池：四档的分析师子集不同（4/5/7/7），且每档回答的是不同持有期的
 * 问题（2 / 5 / 28 / 90 交易日）。合计会把 `market-analyst` 计四票、`policy` 计一票，
 * 并把「2 日看多」与「90 日看多」加进同一个分子；跨档去重又会把另外三档从面板抹掉。
 *
 * 选中档的优先级：用户手动选择 → 本次决策档 → `mid` → 第一个有数据的档。
 * 裸键（分档前的历史快照）单独成组，**不冒充任何一档**。
 *
 * `dataQuality` 传 `data-quality` 节点的产出：本轮方向与多空评分自相矛盾的分析师
 * **不计入票**（判据在后端，前端只读）。扣了必须说 —— `excludedCount` 供呈现层出那句
 * 「N 份报告方向与评分矛盾，未计票」，否则「少一票」会被读成「只有一位分析师有意见」。
 */
export function useAnalystConsensusByTier(
  reports: Record<string, string>,
  decisionTimeHorizon?: string | null,
  dataQuality?: DataQualityReport | null,
) {
  const grouped = useMemo(() => {
    const cleaned: Record<string, string> = {};
    for (const [k, v] of Object.entries(reports)) { cleaned[k] = cleanToolCallTags(v); }
    return groupAnalystReportsByTier(cleaned);
  }, [reports]);

  const options = useMemo<TierConsensus[]>(
    () =>
      Object.entries(grouped.tiers)
        .map(([tier, byBase]) => ({
          tier,
          ...countTally(byBase, contradictionBasesForTier(dataQuality, tier)),
        }))
        // 按权威档序排：store 的键序取决于节点完成先后，切换器不能跟着乱跳
        .sort((a, b) => tierRank(a.tier) - tierRank(b.tier)),
    [grouped, dataQuality],
  );

  const untieredGroup = useMemo(
    () =>
      Object.keys(grouped.untiered).length > 0
        ? countTally(grouped.untiered, contradictionBasesForTier(dataQuality, null))
        : null,
    [grouped, dataQuality],
  );

  const [picked, setPicked] = useState<string | null>(null);
  const activeTier = useMemo(() => {
    if (picked && options.some((o) => o.tier === picked)) { return picked; }
    if (decisionTimeHorizon && options.some((o) => o.tier === decisionTimeHorizon)) {
      return decisionTimeHorizon;
    }
    if (options.some((o) => o.tier === "mid")) { return "mid"; }
    return options[0]?.tier ?? null;
  }, [picked, decisionTimeHorizon, options]);

  const active = options.find((o) => o.tier === activeTier) ?? untieredGroup;
  const tally: AnalystSentimentTally = active?.tally ?? { bullish: 0, bearish: 0, neutral: 0, total: 0 };
  const consensus: Consensus = deriveAnalystConsensus(tally);

  return {
    options,
    untiered: untieredGroup?.tally ?? null,
    activeTier,
    setActiveTier: setPicked,
    tally,
    consensus,
    excludedCount: active?.excluded ?? 0,
  };
}
