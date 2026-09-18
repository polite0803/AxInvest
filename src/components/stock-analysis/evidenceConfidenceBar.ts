// SPDX-License-Identifier: AGPL-3.0-only

/**
 * 证据引用「匹配置信度」条形的渲染判定 —— 从 `EvidenceCitationPanel` 抽出的纯函数。
 *
 * 为什么抽出来：**只是为了可测**。这段逻辑曾经把一次量纲事故静默伪装成正常渲染 ——
 * 后端一度产出 `0–30.7` 的 `matchConfidence`，而这里 `Math.min(x * 100, 100)` 把它压成
 * 「恒满格」、`> 0.5` 又让它「恒绿」，于是缺陷潜伏了很久才被发现。
 * 抽成纯函数后，可以用**历史事故值（4.5）**把它钉死。
 *
 * 契约：`matchConfidence ∈ [0, 1]`（后端类型 `axagent_harness::domain_semantics::Ratio01`，
 * `serde(transparent)` 过线后仍是裸数字）。越界 ⇒ 必须**可见**，不得静默压平。
 */

/** 强/中档分界（与后端 markdown 色带的 0.3 / 0.7 三档保持同源语义） */
const STRONG_THRESHOLD = 0.5;

export const CONFIDENCE_BAR_COLORS = {
  strong: "#22c55e",
  medium: "#eab308",
  outOfRange: "#ef4444",
} as const;

export interface ConfidenceBar {
  /** 条形宽度（百分比，已夹到 0–100）—— 仅用于布局 */
  widthPct: number;
  /** 条形颜色 */
  color: string;
  /** 输入是否越出契约 `[0,1]`（含 `NaN`）⇒ 必须可见 */
  outOfRange: boolean;
}

export function confidenceBar(matchConfidence: number): ConfidenceBar {
  const outOfRange = !Number.isFinite(matchConfidence) || matchConfidence < 0 || matchConfidence > 1;

  // 宽度只服务布局：越界时按方向夹到端点，`NaN` 给 0（不能让 NaN 传播进 CSS）
  const raw = matchConfidence * 100;
  const pct = Number.isFinite(raw) ? raw : matchConfidence > 0 ? 100 : 0;

  return {
    // 宽度仍夹到 0–100（布局不能塌），但越界由 color 暴露 —— 不靠宽度伪装
    widthPct: Math.max(0, Math.min(pct, 100)),
    color: outOfRange
      ? CONFIDENCE_BAR_COLORS.outOfRange
      : matchConfidence > STRONG_THRESHOLD
      ? CONFIDENCE_BAR_COLORS.strong
      : CONFIDENCE_BAR_COLORS.medium,
    outOfRange,
  };
}
