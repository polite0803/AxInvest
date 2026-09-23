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

import type { DataQualityDiagItem, DataQualityReport } from "@/types";

/**
 * 分析师节点 ID → data-quality.rhai 的 `diagnostics` 键名（缩写）。
 *
 * 权威来源：`data-quality.rhai` 头部注释的「分析师缩写对照（当前 DAG 10 个分析师）」。
 * ⚠ DAG 增删分析师时，须同步三处：data-quality.rhai 头部对照表、
 *   其 input_mapping 的 `{abbr}_verdict` 变量、以及本表。
 */
export const EXPERT_ID_TO_DQ_ABBR: Readonly<Record<string, string>> = {
  "a-market-analyst": "mk",
  "a-sentiment": "sent",
  "a-news": "news",
  "a-fundamentals": "fund",
  "a-policy": "pol",
  "a-hot-money": "hm",
  "a-lockup": "lk",
  "a-research": "res",
  "a-sector": "sec",
  "a-catalyst": "cat",
};

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
 *   3. `stale_record === true`（快照缺失时由 store 写入的占位 JSON）
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
  if (rec.stale_record === true) { return null; }
  if (!rec.diagnostics || typeof rec.diagnostics !== "object") { return null; }

  return parsed as DataQualityReport;
}

/**
 * 取某分析师节点的后端诊断条目。
 *
 * @param expertId 节点 ID（如 `a-hot-money`）
 * @param report   已解析的 data-quality 报告
 * @param expertName 可选的中文角色名，用于缩写映射失效时按 name 兜底匹配
 */
export function resolveAnalystDiagnosis(
  expertId: string,
  report: DataQualityReport | null,
  expertName?: string,
): DataQualityDiagItem | null {
  const diags = report?.diagnostics;
  if (!diags) { return null; }

  const abbr = EXPERT_ID_TO_DQ_ABBR[expertId];
  if (abbr && diags[abbr]) { return diags[abbr]; }

  if (expertName) {
    for (const item of Object.values(diags)) {
      if (item?.name === expertName) { return item; }
    }
  }
  return null;
}
