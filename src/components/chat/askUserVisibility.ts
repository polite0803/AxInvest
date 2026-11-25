// SPDX-License-Identifier: AGPL-3.0-only

/**
 * P4-b：从通用提问卡候选里剔除已由持久卡承接的 askId。
 *
 * Bot 线程的提问会被落库并由 `BotInputRequestCard` 承接；同一 askId 的
 * 内存版 `AskUserCard` 必须隐藏，否则同一条提问会出现两张卡。
 * 纯函数，便于在 vitest 中直接钉住「同 askId 不双卡」。
 */
export function pickVisibleAskUsers<T extends { askId: string }>(
  asks: T[],
  hiddenAskIds?: readonly string[],
): T[] {
  if (!hiddenAskIds || hiddenAskIds.length === 0) return asks;
  const hidden = new Set(hiddenAskIds);
  return asks.filter((ask) => !hidden.has(ask.askId));
}
