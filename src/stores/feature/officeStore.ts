// SPDX-License-Identifier: AGPL-3.0-only

import { translateBackendError } from "@/lib/errorI18n";
import { invoke, isTauri } from "@/lib/invoke";
import type {
  AddMemberInput,
  CreateFleetInput,
  DirectMessageInput,
  DispatchEvent,
  DispatchInput,
  Fleet,
  FleetMember,
  FleetMemberStatus,
  FleetMessage,
  FleetStatus,
} from "@/types";
import { CONVERSATION_GROUP, conversationDm, conversationKey } from "@/types";
import { create } from "zustand";

interface OfficeState {
  // ── 数据字段（state）──
  fleets: Fleet[];
  /** 当前选中的舰队 ID（用于 UI 高亮 / 像素办公室渲染） */
  activeFleetId: string | null;
  /** 按舰队 ID 索引的成员列表缓存 */
  membersByFleet: Record<string, FleetMember[]>;
  /** 当前 dispatcher 事件流（Channel 实时追加；**仅过程展示**，不是数据真源） */
  dispatchEvents: DispatchEvent[];
  /**
   * 按**会话**索引的**持久化**消息历史（来自数据库）。
   *
   * 键由 {@link conversationKey} 生成（`fleetId::conversationId`）——
   * 会话隔离是必要的：群聊与某条 DM 各有自己的时间线，混在一起会让
   * DM 内容漏进群聊面板，后端更会把它漏进路由 prompt。
   *
   * 与 `dispatchEvents` 的分工：事件流是「这次发生了什么」的过程回放，
   * 本字段是「会话说过什么」的权威记录。刷新后事件流消失，本字段仍在。
   */
  messagesByConversation: Record<string, FleetMessage[]>;
  loading: boolean;
  error: string | null;

  // ── Actions ──
  loadFleets: (statusFilter?: FleetStatus) => Promise<Fleet[]>;
  selectFleet: (fleetId: string | null) => void;
  createFleet: (input: CreateFleetInput) => Promise<Fleet | null>;
  updateFleetStatus: (fleetId: string, status: FleetStatus) => Promise<void>;
  deleteFleet: (fleetId: string) => Promise<void>;
  loadMembers: (fleetId: string, force?: boolean) => Promise<FleetMember[]>;
  /** 加载指定会话的消息历史（按 seq 升序）；`afterSeq` 用于增量拉取 */
  loadMessages: (
    fleetId: string,
    conversationId: string,
    afterSeq?: number,
  ) => Promise<FleetMessage[]>;
  addMember: (input: AddMemberInput) => Promise<FleetMember | null>;
  updateMemberStatus: (
    memberId: string,
    status: FleetMemberStatus,
  ) => Promise<void>;
  removeMember: (memberId: string, fleetId: string) => Promise<void>;
  resetDailyTokens: (fleetId: string) => Promise<void>;
  dispatch: (input: DispatchInput) => Promise<DispatchEvent[]>;
  directMessage: (
    input: DirectMessageInput,
  ) => Promise<DispatchEvent[]>;
  clearDispatchEvents: () => void;
  clearError: () => void;
}

/**
 * 把一批**持久化**消息按 seq 去重合并进某会话的时间线（升序）。
 *
 * ⚠ **不做覆盖式替换**：本地已有的条目一律保留。
 * 原因有二：
 * 1. 服务端每次只返回**最新 N 条**（`CONVERSATION_HISTORY_LIMIT`）。若替换，
 *    首次加载之外的任何一次刷新都会把本地更长的历史缩回 N 条。
 * 2. HELD 事件会把未读消息**即时并入**本地（见 `applyDispatchEvent`）。
 *    替换式加载会把那批刚展示给用户的消息清掉 —— 「展示即已见」就成了空话。
 *
 * 本表只追加不删除（无删除消息的命令），故合并是安全且无损的。
 */
function mergeMessages(
  set: (fn: (s: OfficeState) => Partial<OfficeState>) => void,
  fleetId: string,
  conversationId: string,
  incoming: FleetMessage[],
): void {
  if (incoming.length === 0) {
    return;
  }
  const key = conversationKey(fleetId, conversationId);
  set((s) => {
    const existing = s.messagesByConversation[key] ?? [];
    const known = new Set(existing.map((m) => m.seq));
    const merged = [...existing, ...incoming.filter((m) => !known.has(m.seq))]
      .sort((a, b) => a.seq - b.seq);
    return { messagesByConversation: { ...s.messagesByConversation, [key]: merged } };
  });
}

/** 把 dispatch 事件追加到事件流，并同步成员状态 / token 用量。 */
function applyDispatchEvent(
  set: (fn: (s: OfficeState) => Partial<OfficeState>) => void,
  get: () => OfficeState,
  evt: DispatchEvent,
  fleetId: string,
  conversationId: string,
): void {
  // 追加事件流
  set((s) => ({ dispatchEvents: [...s.dispatchEvents, evt] }));

  // HELD：服务端已把未读消息内联送来（「展示即已见」）——**立刻**兑现为时间线条目。
  //
  // 为什么不依赖 `dispatch` 收尾的那次重读：那次重读（a）可能失败，
  // （b）只覆盖最新 N 条窗口。把服务端**明确展示过**的消息立刻落到本地，
  // 才与契约字面一致。重复条目由 seq 去重挡掉。
  if (evt.type === "held") {
    mergeMessages(set, fleetId, conversationId, evt.heldMessages);
  }

  // 成员状态 / token 实时回写（驱动 Phaser 精灵动画）
  // 优先按 agentId（memberId）匹配 —— slug 可重复、LLM 路由可能错位，
  // agentId 才是精灵与 FleetMember 一一映射的稳定键；agentSlug 仅作兜底。
  if (evt.type === "agent_status" || evt.type === "token_usage") {
    set((s) => {
      const members = s.membersByFleet[fleetId] ?? [];
      // 存在 agentId 精确匹配时，只更新该成员，避免连带更新同名 slug 成员
      const hasIdMatch = evt.agentId != null && members.some((m) => m.agentId === evt.agentId);
      const next = members.map((m) => {
        const idMatch = evt.agentId != null && m.agentId === evt.agentId;
        const slugMatch = m.agentSlug === evt.agentSlug;
        if (!idMatch && (!slugMatch || hasIdMatch)) {
          return m;
        }
        if (evt.type === "agent_status") {
          return { ...m, status: evt.status };
        }
        return {
          ...m,
          todayTokens: m.todayTokens + evt.inputTokens + evt.outputTokens,
          totalTokens: m.totalTokens + evt.inputTokens + evt.outputTokens,
        };
      });
      return { membersByFleet: { ...s.membersByFleet, [fleetId]: next } };
    });
  }
  void get;
}

export const useOfficeStore = create<OfficeState>((set, get) => ({
  fleets: [],
  activeFleetId: null,
  membersByFleet: {},
  dispatchEvents: [],
  messagesByConversation: {},
  loading: false,
  error: null,

  loadFleets: async (statusFilter?: FleetStatus) => {
    set({ loading: true, error: null });
    try {
      const fleets = await invoke<Fleet[]>("fleet_list", {
        statusFilter: statusFilter ?? null,
      });
      set({ fleets, loading: false });
      return fleets;
    } catch (e) {
      set({ error: String(e), loading: false });
      return [];
    }
  },

  selectFleet: (fleetId) => {
    set({ activeFleetId: fleetId });
  },

  createFleet: async (input) => {
    set({ error: null });
    try {
      const fleet = await invoke<Fleet>("fleet_create", { input });
      set((s) => ({ fleets: [...s.fleets, fleet] }));
      return fleet;
    } catch (e) {
      const msg = translateBackendError(e);
      set({ error: msg });
      console.warn(`[officeStore] createFleet failed: ${msg}`);
      return null;
    }
  },

  updateFleetStatus: async (fleetId, status) => {
    set({ error: null });
    try {
      await invoke<void>("fleet_update_status", { fleetId, status });
      set((s) => ({
        fleets: s.fleets.map((f) => f.id === fleetId ? { ...f, status, updatedAt: Date.now() } : f),
      }));
    } catch (e) {
      set({ error: translateBackendError(e) });
    }
  },

  deleteFleet: async (fleetId) => {
    set({ error: null });
    try {
      await invoke<void>("fleet_delete", { fleetId });
      set((s) => {
        const fleets = s.fleets.filter((f) => f.id !== fleetId);
        const membersByFleet = { ...s.membersByFleet };
        delete membersByFleet[fleetId];
        // 该舰队所有会话（群聊 + 每条 DM）的时间线一并清掉，
        // 键形如 `fleetId::conversationId`，用前缀匹配
        const messagesByConversation = { ...s.messagesByConversation };
        for (const key of Object.keys(messagesByConversation)) {
          if (key.startsWith(`${fleetId}::`)) {
            delete messagesByConversation[key];
          }
        }
        const activeFleetId = s.activeFleetId === fleetId ? null : s.activeFleetId;
        return { fleets, membersByFleet, messagesByConversation, activeFleetId };
      });
    } catch (e) {
      set({ error: translateBackendError(e) });
    }
  },

  loadMembers: async (fleetId, force = false) => {
    set({ error: null });
    // 缓存命中且非强制刷新时直接返回缓存
    if (!force) {
      const cached = get().membersByFleet[fleetId];
      if (cached) {
        return cached;
      }
    }
    try {
      const members = await invoke<FleetMember[]>("fleet_list_members", {
        fleetId,
      });
      set((s) => ({
        membersByFleet: { ...s.membersByFleet, [fleetId]: members },
      }));
      return members;
    } catch (e) {
      set({ error: translateBackendError(e) });
      return [];
    }
  },

  loadMessages: async (fleetId, conversationId, afterSeq) => {
    try {
      // 不传 `afterSeq` 与传 null 语义相同（后端 Option 缺省 None）；
      // 这里按需挂字段，避免依赖 null 的序列化行为
      const params: Record<string, unknown> = { fleetId, conversationId };
      if (afterSeq != null) {
        params.afterSeq = afterSeq;
      }
      const messages = await invoke<FleetMessage[]>("fleet_list_messages", params);
      // 始终按 seq 合并（不覆盖）：服务端只返回最新 N 条，替换会缩回本地历史，
      // 也会清掉 HELD 时即时并入的未读消息。详见 `mergeMessages` 的说明。
      mergeMessages(set, fleetId, conversationId, messages);
      return messages;
    } catch (e) {
      set({ error: translateBackendError(e) });
      return [];
    }
  },

  addMember: async (input) => {
    set({ error: null });
    try {
      const member = await invoke<FleetMember>("fleet_add_member", { input });
      set((s) => {
        const existing = s.membersByFleet[member.fleetId] ?? [];
        return {
          membersByFleet: {
            ...s.membersByFleet,
            [member.fleetId]: [...existing, member],
          },
        };
      });
      return member;
    } catch (e) {
      set({ error: translateBackendError(e) });
      return null;
    }
  },

  updateMemberStatus: async (memberId, status) => {
    set({ error: null });
    try {
      await invoke<void>("fleet_update_member_status", { memberId, status });
      set((s) => {
        const membersByFleet: Record<string, FleetMember[]> = {};
        for (const [fid, members] of Object.entries(s.membersByFleet)) {
          membersByFleet[fid] = members.map((m) => m.id === memberId ? { ...m, status } : m);
        }
        return { membersByFleet };
      });
    } catch (e) {
      set({ error: translateBackendError(e) });
    }
  },

  removeMember: async (memberId, fleetId) => {
    set({ error: null });
    try {
      await invoke<void>("fleet_remove_member", { memberId });
      set((s) => {
        const existing = s.membersByFleet[fleetId] ?? [];
        return {
          membersByFleet: {
            ...s.membersByFleet,
            [fleetId]: existing.filter((m) => m.id !== memberId),
          },
        };
      });
    } catch (e) {
      set({ error: translateBackendError(e) });
    }
  },

  resetDailyTokens: async (fleetId) => {
    set({ error: null });
    try {
      await invoke<void>("fleet_reset_daily_tokens", { fleetId });
      set((s) => {
        const existing = s.membersByFleet[fleetId] ?? [];
        return {
          membersByFleet: {
            ...s.membersByFleet,
            [fleetId]: existing.map((m) => ({ ...m, todayTokens: 0 })),
          },
        };
      });
    } catch (e) {
      set({ error: translateBackendError(e) });
    }
  },

  dispatch: async (input) => {
    set({ error: null, dispatchEvents: [] });
    const fleetId = input.fleetId;
    // 群聊路径固定落在群聊会话
    const conversationId = CONVERSATION_GROUP;
    try {
      // 事件回传：Tauri 用 Channel（流式）；浏览器模式用 MockChannel 普通对象
      // （Tauri Channel 构造依赖 __TAURI_INTERNALS__.transformCallback，浏览器模式会抛错）
      if (isTauri()) {
        const { Channel } = await import("@tauri-apps/api/core");
        const channel = new Channel<DispatchEvent>();
        channel.onmessage = (evt) => {
          applyDispatchEvent(set, get, evt, fleetId, conversationId);
        };
        await invoke<void>("fleet_dispatch", { input, onEvent: channel });
      } else {
        const mockChannel: { onmessage: (evt: DispatchEvent) => void } = {
          onmessage: (evt) => {
            applyDispatchEvent(set, get, evt, fleetId, conversationId);
          },
        };
        await invoke<void>("fleet_dispatch", { input, onEvent: mockChannel });
      }
      return get().dispatchEvents;
    } catch (e) {
      const errorEvent: DispatchEvent = {
        type: "error",
        message: translateBackendError(e),
      };
      applyDispatchEvent(set, get, errorEvent, fleetId, conversationId);
      return get().dispatchEvents;
    } finally {
      // 无论成功 / 被暂扣 / 失败，都从库重读一次：用户消息可能已落库，
      // agent 回复也可能已落库 —— 事件流是过程回放，不能当作历史。
      await get().loadMessages(fleetId, conversationId);
    }
  },

  directMessage: async (input) => {
    set({ error: null, dispatchEvents: [] });
    const fleetId = input.fleetId;
    // 私信路径落在**该成员专属**的会话上 —— 与群聊、与别人的私信都互不可见
    const conversationId = conversationDm(input.agentSlug);
    try {
      if (isTauri()) {
        const { Channel } = await import("@tauri-apps/api/core");
        const channel = new Channel<DispatchEvent>();
        channel.onmessage = (evt) => {
          applyDispatchEvent(set, get, evt, fleetId, conversationId);
        };
        await invoke<void>("fleet_direct_message", { input, onEvent: channel });
      } else {
        const mockChannel: { onmessage: (evt: DispatchEvent) => void } = {
          onmessage: (evt) => {
            applyDispatchEvent(set, get, evt, fleetId, conversationId);
          },
        };
        await invoke<void>("fleet_direct_message", { input, onEvent: mockChannel });
      }
      return get().dispatchEvents;
    } catch (e) {
      const errorEvent: DispatchEvent = {
        type: "error",
        message: translateBackendError(e),
      };
      applyDispatchEvent(set, get, errorEvent, fleetId, conversationId);
      return get().dispatchEvents;
    } finally {
      // 同 dispatch：以库为准刷新历史
      await get().loadMessages(fleetId, conversationId);
    }
  },

  clearDispatchEvents: () => {
    set({ dispatchEvents: [] });
  },

  clearError: () => {
    set({ error: null });
  },
}));
