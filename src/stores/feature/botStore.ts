// SPDX-License-Identifier: AGPL-3.0-only

// Bot Store —— Bot 归属层的列表 / CRUD / SOUL / 记忆 / 线程读取
//
// 命令契约冻结在 `docs/bot-integration/IPC.md`（9 个命令、11 个错误码）：
// 参数一律 camelCase（Tauri v2 默认 `rename_all = "camelCase"`），
// 错误经 `backendErrorText` 翻译为 `error.<CODE>` 文案后落到 `error`。
//
// 分层：本文件属 feature 层。`components/bot/**` 可直读本 store；
// `components/chat/**` 禁止引用（跨模块走 `pages/**` 的 props 注入）。

import { backendErrorText } from "@/lib/errorI18n";
import { invoke } from "@/lib/invoke";
import type { Bot, BotMemory, BotSoul, Conversation, CreateBotInput, DeleteBotResult, UpdateBotInput } from "@/types";
import { create } from "zustand";

export interface BotStoreState {
  /** 侧栏 / 列表数据源（后端已按 sortOrder 排序） */
  bots: Bot[];
  loading: boolean;
  /** 创建 / 更新的在途标记 */
  saving: boolean;
  /** 删除的在途标记 */
  deleting: boolean;
  /** 最近一次失败的本地化文案（null = 无错误） */
  error: string | null;
  /** 当前选中的 Bot（详情面板与新建线程的归属来源） */
  activeBotId: string | null;

  loadBots: (includeDisabled?: boolean) => Promise<Bot[]>;
  setActiveBot: (botId: string | null) => void;
  /** 单个 Bot 的最新快照（编辑前刷新，避免列表快照过期） */
  getBot: (botId: string) => Promise<Bot>;
  createBot: (input: CreateBotInput) => Promise<Bot>;
  updateBot: (botId: string, input: UpdateBotInput) => Promise<Bot>;
  deleteBot: (botId: string) => Promise<DeleteBotResult>;
  /** 只读：SOUL 正文 + hash + 漂移标记 */
  readSoul: (botId: string) => Promise<BotSoul>;
  /** 写 DB + 原子重写镜像；成功后同步列表里的 soul / hash / drift */
  writeSoul: (botId: string, content: string) => Promise<BotSoul>;
  /** 只读：MEMORY.md 索引 + 预算 + 主题清单 */
  readMemory: (botId: string) => Promise<BotMemory>;
  /** 只读：该 Bot 的线程列表（直接返回既有 Conversation DTO） */
  listThreads: (botId: string, includeArchived?: boolean) => Promise<Conversation[]>;
}

/** 防止并发 loadBots 的旧响应覆盖新响应（切换 includeDisabled 时的高发点）。 */
let loadSeq = 0;

export const useBotStore = create<BotStoreState>((set) => ({
  bots: [],
  loading: false,
  saving: false,
  deleting: false,
  error: null,
  activeBotId: null,

  loadBots: async (includeDisabled = false) => {
    const seq = ++loadSeq;
    set({ loading: true, error: null });
    try {
      const bots = await invoke<Bot[]>("list_bots", { includeDisabled });
      if (seq === loadSeq) {
        set({ bots, loading: false });
      }
      return bots;
    } catch (e) {
      if (seq === loadSeq) {
        set({ loading: false, error: backendErrorText(e) });
      }
      throw e;
    }
  },

  setActiveBot: (botId) => {
    set({ activeBotId: botId });
  },

  getBot: async (botId) => {
    const bot = await invoke<Bot>("get_bot", { botId });
    set((s) => ({
      bots: s.bots.some((b) => b.id === bot.id) ? s.bots.map((b) => (b.id === bot.id ? bot : b)) : [bot, ...s.bots],
    }));
    return bot;
  },

  createBot: async (input) => {
    set({ saving: true, error: null });
    try {
      const bot = await invoke<Bot>("create_bot", { input });
      set((s) => ({
        bots: [bot, ...s.bots.filter((b) => b.id !== bot.id)],
        activeBotId: bot.id,
        saving: false,
      }));
      return bot;
    } catch (e) {
      set({ saving: false, error: backendErrorText(e) });
      throw e;
    }
  },

  updateBot: async (botId, input) => {
    set({ saving: true, error: null });
    try {
      const bot = await invoke<Bot>("update_bot", { botId, input });
      set((s) => ({
        bots: s.bots.map((b) => (b.id === bot.id ? bot : b)),
        saving: false,
      }));
      return bot;
    } catch (e) {
      set({ saving: false, error: backendErrorText(e) });
      throw e;
    }
  },

  deleteBot: async (botId) => {
    set({ deleting: true, error: null });
    try {
      const result = await invoke<DeleteBotResult>("delete_bot", { botId });
      set((s) => ({
        bots: s.bots.filter((b) => b.id !== botId),
        activeBotId: s.activeBotId === botId ? null : s.activeBotId,
        deleting: false,
      }));
      return result;
    } catch (e) {
      set({ deleting: false, error: backendErrorText(e) });
      throw e;
    }
  },

  readSoul: (botId) => invoke<BotSoul>("read_bot_soul", { botId }),

  writeSoul: async (botId, content) => {
    set({ saving: true, error: null });
    try {
      const soul = await invoke<BotSoul>("write_bot_soul", { botId, content });
      set((s) => ({
        saving: false,
        bots: s.bots.map((b) =>
          b.id === botId ? { ...b, soul: soul.content, soulHash: soul.hash, soulDrift: soul.drift } : b
        ),
      }));
      return soul;
    } catch (e) {
      set({ saving: false, error: backendErrorText(e) });
      throw e;
    }
  },

  readMemory: (botId) => invoke<BotMemory>("read_bot_memory", { botId }),

  listThreads: (botId, includeArchived = false) =>
    invoke<Conversation[]>("list_bot_threads", { botId, includeArchived }),
}));
