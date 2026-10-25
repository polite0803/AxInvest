// i18n-exempt: 数据质量诊断的节点映射与结构化字段解析逻辑，技术字符串，非用户可见 UI 文案。
//
// 数据质量诊断（前端只读视图）
//
// 设计原则（2026-09-14 重构）：
//   此前「决策链数据质量」（后端 data-quality.rhai）与「分析师卡片数据质量弹窗」
//   （前端 analyzeDataQuality）是**两套完全独立的算法**，同名同字母等级却口径不同：
//     后端：聚合 10 份报告正文 + 9 个因子 + 工具可信度，阈值 A≥85/B≥65/C≥45/D≥25
//     前端：只看单个节点的 JSON 结构，阈值 A≥90/B≥70/C≥50/D≥30，且不读报告正文
//   后果：一份正文写着「无法获取，数据缺失」但 JSON 字段齐全的报告，弹窗判 A，
//   而后端因命中失败标记判该节点降级、拉低全局至 C —— 用户看到两个矛盾等级。
//   现前端不再自算，改为直接消费 data-quality.rhai 的权威输出（grade/score/diagnostics），
//   全项目只保留一套数据质量判定。

import { analystBaseOf } from "@/lib/stock-analysis-utils";
import type { DataQualityDiagItem, DataQualityReport } from "@/types";

/**
 * 分析师节点 ID → data-quality.rhai 的 `diagnostics` 键名（缩写）。
 *
 * 权威来源：`data-quality.rhai` 头部注释的「分析师缩写对照（10 个槽位）」。
 * ⚠ DAG 增删分析师时，须同步三处：data-quality.rhai 头部对照表、
 *   其 input_mapping 的 `{abbr}_verdict` 变量、以及本表。
 * v133（B2-2）：a-news 退役（公告通道由 a-catalyst 承载）；value-investor 上线（槽位 `val`）。
 * 键域是**裸节点 id**——调用方若是逐档实例（`a-market-analyst--mid`），
 * `resolveAnalystDiagnosis` 会先归一再查。
 */
export const EXPERT_ID_TO_DQ_ABBR: Readonly<Record<string, string>> = {
  "a-market-analyst": "mk",
  "a-sentiment": "sent",
  "a-fundamentals": "fund",
  "a-policy": "pol",
  "a-hot-money": "hm",
  "a-lockup": "lk",
  "a-research": "res",
  "a-sector": "sec",
  "a-catalyst": "cat",
  "value-investor": "val",
};

/** 反向表：由上表派生，**不是第二份登记**（新增分析师只改 `EXPERT_ID_TO_DQ_ABBR` 一处）。 */
export const DQ_ABBR_TO_NODE_ID: Readonly<Record<string, string>> = Object.fromEntries(
  Object.entries(EXPERT_ID_TO_DQ_ABBR).map(([nodeId, abbr]) => [abbr, nodeId]),
);

/**
 * 把任意形态的分析师标识归一到 `data-quality` 的槽位缩写。
 *
 * 存在理由（2026-10-25 实测）：本表的键域是**节点 id**（`a-market-analyst`），而 store 落
 * `analystReports` 的键是**去掉 `a-` 前缀**的 base（`market-analyst`）⇒ 原先直接
 * `EXPERT_ID_TO_DQ_ABBR[base]` 时 10 个分析师**全部精确查表落空**，只能靠「比对界面译文」
 * 那条兜底救回 4 个（mk / sent / val / pol / res / cat 六个连兜底都不中，因为后端诊断名
 * 写的是「技术面分析师」而界面译文是「技术分析师」）。查表恒落空、被兜底掩盖成看起来正常，
 * 与本仓权重表同一个失效形状，故在查表这一步就把两种键形态都认。
 */
export function dqAbbrOf(nodeIdOrBase: string): string | undefined {
  const base = analystBaseOf(nodeIdOrBase) ?? nodeIdOrBase;
  return EXPERT_ID_TO_DQ_ABBR[base] ?? EXPERT_ID_TO_DQ_ABBR[`a-${base}`];
}

/**
 * 节点 id → store 落 `analystReports` 用的键形态（去掉 `a-` 前缀）。
 * 剥前缀这件事只在这里做一次；两边各写一份 `replace(/^a-/,"")` 迟早一边改一边不改。
 */
export function analystStoreKeyOf(nodeId: string): string {
  return nodeId.replace(/^a-/, "");
}

/** 后端诊断状态 → 面板展示用的严重度 */
export type DiagSeverity = "good" | "warning" | "issue";

/**
 * 后端 status 映射为展示严重度。
 *   normal    正常（置信度 ≥50 且报告无失败标记）
 *   low       低置信（自评 <50，或报告含失败标记 ⇒ 置信度虚高）
 *   missing   节点无输出 / confidence 字段缺失
 *   untrusted strict_mode 降级兜底（LLM 不可用，confidence 为中性 50）
 */
export function diagStatusToSeverity(status: DataQualityDiagItem["status"]): DiagSeverity {
  switch (status) {
    case "normal":
      return "good";
    case "low":
      return "warning";
    default:
      return "issue";
  }
}

/**
 * 单节点报告质量分（`report_quality`，0-100）→ 展示严重度。
 *
 * 2026-09-21 抽为**单一来源**：此前该三档阈值只写在 `AnalystDataQualityModal` 的行内表达式里；
 * 同日又给 `DecisionBanner` 的逐节点表格加「报告质量」列，若各写一份就会出现
 * **同一个数值在两处显示不同颜色** —— 正是本项目当天全仓清理的「同语义多份判据」形态。
 *
 * ⚠ 这是**视觉分层**（快速指示），**不是**第二套等级体系：全工作流的唯一权威等级仍是
 *   `data-quality.rhai` 输出的 `grade`（A–F）。禁止据此派生 per-node 字母等级 ——
 *   那正是 2026-09-14 才修掉的坑（见本文件头部说明）。
 *
 * ⚠ 返回 `null` 表示**无值**（旧版快照没有 `report_quality` 字段），调用方须降级展示。
 *   刻意不把「无值」并入 `good`：那会把「不知道」渲染成「好」（本次修正的一处旧行为）。
 *
 * 注：两处调用方各自保留既有调色板（modal 用 antd 语义色、DecisionBanner 用 tailwind 色阶），
 *   本函数只出**档位**不出颜色 —— 同一档在两处的色值本就不同，不要误以为能直接复用色值。
 *
 * @param rq 节点的 `report_quality`；`undefined` = 旧版快照无此字段（**不能当 0**，0 是合法取值）
 */
export function reportQualitySeverity(rq: number | undefined): DiagSeverity | null {
  if (rq === undefined) { return null; }
  if (rq >= 80) { return "good"; }
  if (rq >= 50) { return "warning"; }
  return "issue";
}

/**
 * 解析 store 中的 `dataQualitySummary`（data-quality 节点输出的 JSON 字符串）。
 *
 * 返回 null 的四种情况（调用方应据此降级展示，而不是自行计算）：
 *   1. 空字符串 / 非法 JSON
 *   2. 缺少 `grade` 字段（不是 data-quality 节点的输出）
 *   3. 带 `snapshot_absence`（无黑板快照时由 store 写入的占位 JSON；呈现层按该键出缺席文案）
 *   4. 缺少 `diagnostics`（2026-07-23 之前的旧版快照，无法定位到单个分析师）
 */
export function parseDataQualityReport(raw: string | null | undefined): DataQualityReport | null {
  if (!raw || !raw.trim()) { return null; }

  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch {
    return null;
  }
  if (!parsed || typeof parsed !== "object") { return null; }

  const rec = parsed as Record<string, unknown>;
  if (typeof rec.grade !== "string") { return null; }
  if (typeof rec.snapshot_absence === "string") { return null; }
  if (!rec.diagnostics || typeof rec.diagnostics !== "object") { return null; }

  return parsed as DataQualityReport;
}

/**
 * 取某分析师节点的后端诊断条目。
 *
 * @param expertId 任意形态的分析师标识（节点 id `a-hot-money`、store 键 `hot-money`、
 *                 或逐档实例 `a-market-analyst--mid` / `market-analyst--mid` 都认）
 * @param report   已解析的 data-quality 报告
 * @param expertName 仅用作**末级兜底**（见下方注释，不能当主路）
 * @param tier     本次要看的持有期；给了且该档有行 ⇒ 用该档那一行
 */
export function resolveAnalystDiagnosis(
  expertId: string,
  report: DataQualityReport | null,
  expertName?: string,
  tier?: string | null,
): DataQualityDiagItem | null {
  const diags = report?.diagnostics;
  if (!diags) { return null; }

  const abbr = dqAbbrOf(expertId);
  if (abbr) {
    // 逐档实例化（v133）后，同一缩写在各档各有一行；指明档位时用本档那一行，
    // 否则 `技术面·超短` 的弹窗会显示 mid 代表实例的结论（同屏两个数字对不上）。
    const rows = report?.diagnostics_by_tier?.[abbr];
    const atTier = tier ? rows?.[tier] : undefined;
    if (atTier) { return atTier; }
    if (diags[abbr]) { return diags[abbr]; }
  }

  // 末级兜底：按后端硬编码的中文角色名比对。它只能救「新增分析师还没登记进
  // `EXPERT_ID_TO_DQ_ABBR`」这一种情况，且**只在中文界面成立**（译文一变就全落空），
  // 所以不得当作主路 —— 主路是上面那个归一化查表。
  if (expertName) {
    for (const item of Object.values(diags)) {
      if (item?.name === expertName) { return item; }
    }
  }
  return null;
}

/**
 * 本轮「结论方向与多空评分互相打脸」的分析师集合，键是 **store 落 `analystReports` 时的那种 base**
 * （去掉 `a-` 前缀，如 `market-analyst`），便于调用方直接与逐档报告键比对。
 *
 * 判据住在后端 `data-quality.rhai::verdict_direction_contradiction`，前端只读结论、不重算
 * （禁区 12：同一逻辑不许有第二份实现）。
 *
 * 三种情况必须分开（「结构性缺口不得在 UI 造成歧义」那条纪律）：
 *   · `null` = 诊断里**没有** `inconsistent_reason` 这个字段（旧快照 / 未重播种）⇒ 这是**不知道**，
 *     调用方一律照常计票，绝不能把「不知道」当成「无人矛盾」以外的任何结论；
 *   · 空集合 = 字段在、本档无人矛盾；
 *   · 非空 = 这些分析师本档不计票，且呈现层必须说清少了几票、为什么少。
 *
 * 取数优先逐档行（`diagnostics_by_tier[abbr][tier]`），该档没行时回落代表实例
 * （`diagnostics[abbr]`）—— 与后端「grade 只看代表实例」同一取舍，不是新口径。
 */
export function contradictionBasesForTier(
  report: DataQualityReport | null | undefined,
  tier: string | null | undefined,
): Set<string> | null {
  const diags = report?.diagnostics;
  const byTier = report?.diagnostics_by_tier;
  if (!diags && !byTier) { return null; }

  // 「知不知道」= 有没有任何一行带上了这个字段。全都缺 ⇒ 旧快照，无从判断。
  const knows = Object.values(diags ?? {}).some((d) => typeof d?.inconsistent_reason === "string")
    || Object.values(byTier ?? {}).some(
      (rows) => Object.values(rows ?? {}).some((d) => typeof d?.inconsistent_reason === "string"),
    );
  if (!knows) { return null; }

  const out = new Set<string>();
  for (const [abbr, nodeId] of Object.entries(DQ_ABBR_TO_NODE_ID)) {
    const item = byTier?.[abbr]?.[tier ?? ""] ?? diags?.[abbr];
    if (item && (item.inconsistent_reason ?? "") !== "") {
      out.add(analystStoreKeyOf(nodeId));
    }
  }
  return out;
}
