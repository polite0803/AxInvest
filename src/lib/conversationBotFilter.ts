// SPDX-License-Identifier: AGPL-3.0-only

/**
 * 会话按 Bot 过滤 —— 聊天侧栏 Bot 上下文的**唯一**判定实现。
 *
 * 为什么抽成纯函数：`ChatSidebar` 有 2400+ 行，直接在里面写三行 filter 无法单测，
 * 而「按 Bot 过滤后恒空」正是本仓 e2e/mock 最容易假绿的一类缺陷
 * （见 `PLAN-bot-integration` T4「风险 2」）。纯函数让两种边界（未归属开关、空 Bot 上下文）
 * 有确定性断言。
 *
 * 语义：
 *   - `activeBotId` 为空 ⇒ **不过滤**（非 Bot 上下文行为与升级前逐字一致）；
 *   - `activeBotId` 非空 ⇒ 只保留 `botId === activeBotId` 的会话；
 *   - `includeUnassigned` 为真时额外保留 `botId == null`（「未归属」历史线程）。
 */

/** 判定所需的最小会话形状（`Conversation.botId` 为可空字段）。 */
export interface BotOwnedConversation {
  botId?: string | null;
}

export function filterConversationsByBot<T extends BotOwnedConversation>(
  conversations: readonly T[],
  activeBotId: string | null | undefined,
  includeUnassigned = false,
): T[] {
  if (!activeBotId) {
    return [...conversations];
  }
  return conversations.filter((c) => c.botId === activeBotId || (includeUnassigned && c.botId == null));
}
