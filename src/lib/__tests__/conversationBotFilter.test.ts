// SPDX-License-Identifier: AGPL-3.0-only

import { type BotOwnedConversation, filterConversationsByBot } from "@/lib/conversationBotFilter";
import { describe, expect, it } from "vitest";

interface Conv extends BotOwnedConversation {
  id: string;
}

const CONVERSATIONS: Conv[] = [
  { id: "a1", botId: "bot-a" },
  { id: "a2", botId: "bot-a" },
  { id: "b1", botId: "bot-b" },
  { id: "orphan-undefined" },
  { id: "orphan-null", botId: null },
];

describe("filterConversationsByBot", () => {
  it("无 Bot 上下文时不过滤，且返回新数组（不改动调用方数据）", () => {
    const all = filterConversationsByBot(CONVERSATIONS, null);
    expect(all.map((c) => c.id)).toEqual(CONVERSATIONS.map((c) => c.id));
    expect(all).not.toBe(CONVERSATIONS);

    expect(filterConversationsByBot(CONVERSATIONS, undefined)).toHaveLength(CONVERSATIONS.length);
    expect(filterConversationsByBot(CONVERSATIONS, "")).toHaveLength(CONVERSATIONS.length);
  });

  it("给定 Bot 时只保留该 Bot 的线程", () => {
    expect(filterConversationsByBot(CONVERSATIONS, "bot-a").map((c) => c.id)).toEqual(["a1", "a2"]);
    expect(filterConversationsByBot(CONVERSATIONS, "bot-b").map((c) => c.id)).toEqual(["b1"]);
  });

  it("Bot 没有线程时返回空数组，不混入其它 Bot 或未归属线程", () => {
    expect(filterConversationsByBot(CONVERSATIONS, "bot-c")).toEqual([]);
  });

  it("includeUnassigned 额外保留 botId 缺失或为 null 的历史线程", () => {
    expect(filterConversationsByBot(CONVERSATIONS, "bot-a", true).map((c) => c.id)).toEqual([
      "a1",
      "a2",
      "orphan-undefined",
      "orphan-null",
    ]);
  });

  it("空列表不报错", () => {
    expect(filterConversationsByBot([], "bot-a")).toEqual([]);
    expect(filterConversationsByBot([], null)).toEqual([]);
  });
});
