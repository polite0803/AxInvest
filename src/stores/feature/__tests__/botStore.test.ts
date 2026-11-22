// SPDX-License-Identifier: AGPL-3.0-only

/**
 * botStore 契约测试。
 *
 * 命令名 / 参数键冻结在 `docs/bot-integration/IPC.md`（9 个命令），
 * 这里逐条钉住「命令名 + camelCase 参数」，防止前端悄悄改名或改形。
 */

import type { Bot, BotMemory, BotSoul, Conversation, DeleteBotResult } from "@/types";
import { beforeEach, describe, expect, it, vi } from "vitest";

const { invokeMock } = vi.hoisted(() => ({
  invokeMock: vi.fn(),
}));

vi.mock("@/lib/invoke", () => ({
  invoke: invokeMock,
  isTauri: () => false,
}));

import { useBotStore } from "@/stores/feature/botStore";

function makeBot(overrides: Partial<Bot> = {}): Bot {
  return {
    id: "bot-1",
    name: "投研助手",
    title: "A 股",
    description: null,
    soul: "# SOUL",
    soulHash: "deadbeef",
    soulDrift: false,
    icon: "🤖",
    color: null,
    section: "research",
    modelId: null,
    providerId: null,
    effort: null,
    workdir: null,
    approvalMode: "full",
    agentProfileId: null,
    isEnabled: true,
    sortOrder: 0,
    createdAt: 1_700_000_000,
    updatedAt: 1_700_000_000,
    ...overrides,
  };
}

/** 只填断言用到的字段：`list_bot_threads` 直接返回既有 Conversation DTO。 */
function makeConversation(overrides: Partial<Conversation> = {}): Conversation {
  return { id: "conv-1", title: "线程", botId: "bot-1", ...overrides } as unknown as Conversation;
}

describe("botStore", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    useBotStore.setState({
      bots: [],
      loading: false,
      saving: false,
      deleting: false,
      error: null,
      activeBotId: null,
    });
  });

  describe("loadBots", () => {
    it("默认跳过禁用 Bot，并把结果写入 store", async () => {
      const bots = [makeBot()];
      invokeMock.mockResolvedValueOnce(bots);

      const result = await useBotStore.getState().loadBots();

      expect(invokeMock).toHaveBeenCalledWith("list_bots", { includeDisabled: false });
      expect(result).toEqual(bots);
      expect(useBotStore.getState().bots).toEqual(bots);
      expect(useBotStore.getState().loading).toBe(false);
      expect(useBotStore.getState().error).toBeNull();
    });

    it("includeDisabled 透传给后端", async () => {
      invokeMock.mockResolvedValueOnce([]);

      await useBotStore.getState().loadBots(true);

      expect(invokeMock).toHaveBeenCalledWith("list_bots", { includeDisabled: true });
    });

    it("失败时记录本地化错误并复位 loading，同时把异常抛给调用方", async () => {
      invokeMock.mockRejectedValueOnce(new Error("boom"));

      await expect(useBotStore.getState().loadBots()).rejects.toThrow("boom");

      expect(useBotStore.getState().loading).toBe(false);
      expect(useBotStore.getState().error).toBeTruthy();
      expect(useBotStore.getState().bots).toEqual([]);
    });

    it("并发加载时旧响应不得覆盖新响应", async () => {
      let resolveFirst: (bots: Bot[]) => void = () => {};
      let resolveSecond: (bots: Bot[]) => void = () => {};
      invokeMock
        .mockImplementationOnce(() => new Promise<Bot[]>((resolve) => (resolveFirst = resolve)))
        .mockImplementationOnce(() => new Promise<Bot[]>((resolve) => (resolveSecond = resolve)));

      const first = useBotStore.getState().loadBots();
      const second = useBotStore.getState().loadBots();

      resolveSecond([makeBot({ id: "new", name: "新" })]);
      await second;
      resolveFirst([makeBot({ id: "old", name: "旧" })]);
      await first;

      expect(useBotStore.getState().bots.map((b) => b.id)).toEqual(["new"]);
      expect(useBotStore.getState().loading).toBe(false);
    });
  });

  describe("setActiveBot", () => {
    it("写入当前 Bot，可置空", () => {
      useBotStore.getState().setActiveBot("bot-1");
      expect(useBotStore.getState().activeBotId).toBe("bot-1");

      useBotStore.getState().setActiveBot(null);
      expect(useBotStore.getState().activeBotId).toBeNull();
    });
  });

  describe("getBot", () => {
    it("以 get_bot 拉最新快照并就地更新列表", async () => {
      useBotStore.setState({ bots: [makeBot({ id: "bot-1", name: "旧" })] });
      invokeMock.mockResolvedValueOnce(makeBot({ id: "bot-1", name: "新" }));

      const bot = await useBotStore.getState().getBot("bot-1");

      expect(invokeMock).toHaveBeenCalledWith("get_bot", { botId: "bot-1" });
      expect(bot.name).toBe("新");
      expect(useBotStore.getState().bots.map((b) => b.name)).toEqual(["新"]);
    });

    it("列表里没有该 Bot 时插入到首位", async () => {
      useBotStore.setState({ bots: [makeBot({ id: "bot-2" })] });
      invokeMock.mockResolvedValueOnce(makeBot({ id: "bot-9" }));

      await useBotStore.getState().getBot("bot-9");

      expect(useBotStore.getState().bots.map((b) => b.id)).toEqual(["bot-9", "bot-2"]);
    });
  });

  describe("writeSoul", () => {
    it("调用 write_bot_soul，并把写的正文/哈希/漂移同步回列表", async () => {
      useBotStore.setState({ bots: [makeBot({ id: "bot-1", soul: "旧", soulHash: "old", soulDrift: true })] });
      invokeMock.mockResolvedValueOnce({ content: "新", hash: "new", drift: false } satisfies BotSoul);

      const soul = await useBotStore.getState().writeSoul("bot-1", "新");

      expect(invokeMock).toHaveBeenCalledWith("write_bot_soul", { botId: "bot-1", content: "新" });
      expect(soul).toEqual({ content: "新", hash: "new", drift: false });
      const updated = useBotStore.getState().bots[0];
      expect(updated.soul).toBe("新");
      expect(updated.soulHash).toBe("new");
      expect(updated.soulDrift).toBe(false);
      expect(useBotStore.getState().saving).toBe(false);
    });

    it("写入失败时复位 saving 并保留原 SOUL", async () => {
      useBotStore.setState({ bots: [makeBot({ id: "bot-1", soul: "旧" })] });
      invokeMock.mockRejectedValueOnce(new Error("BOT_SOUL_WRITE_FAILED"));

      await expect(useBotStore.getState().writeSoul("bot-1", "新")).rejects.toThrow("BOT_SOUL_WRITE_FAILED");

      expect(useBotStore.getState().saving).toBe(false);
      expect(useBotStore.getState().bots[0].soul).toBe("旧");
      expect(useBotStore.getState().error).toBeTruthy();
    });
  });

  describe("createBot", () => {
    it("以 input 包一层调用 create_bot，插入列表首位并自动选中", async () => {
      const created = makeBot({ id: "bot-new", name: "新机器人" });
      useBotStore.setState({ bots: [makeBot({ id: "bot-0" })], activeBotId: "bot-0" });
      invokeMock.mockResolvedValueOnce(created);

      const result = await useBotStore.getState().createBot({ name: "新机器人", approvalMode: "ask" });

      expect(invokeMock).toHaveBeenCalledWith("create_bot", {
        input: { name: "新机器人", approvalMode: "ask" },
      });
      expect(result).toEqual(created);
      expect(useBotStore.getState().bots.map((b) => b.id)).toEqual(["bot-new", "bot-0"]);
      expect(useBotStore.getState().activeBotId).toBe("bot-new");
      expect(useBotStore.getState().saving).toBe(false);
    });

    it("失败时复位 saving 并抛错", async () => {
      invokeMock.mockRejectedValueOnce(new Error("BOT_NAME_INVALID"));

      await expect(useBotStore.getState().createBot({ name: "" })).rejects.toThrow("BOT_NAME_INVALID");

      expect(useBotStore.getState().saving).toBe(false);
      expect(useBotStore.getState().error).toBeTruthy();
    });
  });

  describe("updateBot", () => {
    it("以 { botId, input } 调用 update_bot 并就地替换", async () => {
      useBotStore.setState({ bots: [makeBot({ id: "bot-1", name: "旧" }), makeBot({ id: "bot-2" })] });
      const updated = makeBot({ id: "bot-1", name: "新" });
      invokeMock.mockResolvedValueOnce(updated);

      await useBotStore.getState().updateBot("bot-1", { name: "新" });

      expect(invokeMock).toHaveBeenCalledWith("update_bot", { botId: "bot-1", input: { name: "新" } });
      expect(useBotStore.getState().bots[0].name).toBe("新");
      expect(useBotStore.getState().bots[1].id).toBe("bot-2");
    });
  });

  describe("deleteBot", () => {
    it("调用 delete_bot，移除条目、清空同 id 的选中项，并返回解绑统计", async () => {
      const result: DeleteBotResult = { detachedThreads: 2, memoryPath: "/mem/bot-1/MEMORY.md" };
      useBotStore.setState({ bots: [makeBot({ id: "bot-1" }), makeBot({ id: "bot-2" })], activeBotId: "bot-1" });
      invokeMock.mockResolvedValueOnce(result);

      const returned = await useBotStore.getState().deleteBot("bot-1");

      expect(invokeMock).toHaveBeenCalledWith("delete_bot", { botId: "bot-1" });
      expect(returned).toEqual(result);
      expect(useBotStore.getState().bots.map((b) => b.id)).toEqual(["bot-2"]);
      expect(useBotStore.getState().activeBotId).toBeNull();
      expect(useBotStore.getState().deleting).toBe(false);
    });

    it("删除的不是当前选中 Bot 时保留选中项", async () => {
      useBotStore.setState({ bots: [makeBot({ id: "bot-1" }), makeBot({ id: "bot-2" })], activeBotId: "bot-1" });
      invokeMock.mockResolvedValueOnce({ detachedThreads: 0, memoryPath: "" });

      await useBotStore.getState().deleteBot("bot-2");

      expect(useBotStore.getState().activeBotId).toBe("bot-1");
    });
  });

  describe("只读命令", () => {
    it("readSoul 调用 read_bot_soul", async () => {
      const soul: BotSoul = { content: "# hi", hash: "abcd1234", drift: false };
      invokeMock.mockResolvedValueOnce(soul);

      await expect(useBotStore.getState().readSoul("bot-1")).resolves.toEqual(soul);
      expect(invokeMock).toHaveBeenCalledWith("read_bot_soul", { botId: "bot-1" });
    });

    it("readMemory 调用 read_bot_memory", async () => {
      const memory: BotMemory = {
        indexContent: "# MEMORY",
        lineCount: 1,
        byteCount: 8,
        budget: { maxLines: 200, maxBytes: 24576 },
        topics: [],
      };
      invokeMock.mockResolvedValueOnce(memory);

      await expect(useBotStore.getState().readMemory("bot-1")).resolves.toEqual(memory);
      expect(invokeMock).toHaveBeenCalledWith("read_bot_memory", { botId: "bot-1" });
    });

    it("listThreads 调用 list_bot_threads，归档开关默认 false", async () => {
      const threads = [makeConversation()];
      invokeMock.mockResolvedValueOnce(threads);

      await expect(useBotStore.getState().listThreads("bot-1")).resolves.toEqual(threads);
      expect(invokeMock).toHaveBeenCalledWith("list_bot_threads", { botId: "bot-1", includeArchived: false });

      invokeMock.mockResolvedValueOnce([]);
      await useBotStore.getState().listThreads("bot-1", true);
      expect(invokeMock).toHaveBeenLastCalledWith("list_bot_threads", { botId: "bot-1", includeArchived: true });
    });
  });
});
