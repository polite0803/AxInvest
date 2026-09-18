// SPDX-License-Identifier: AGPL-3.0-only

/**
 * officeStore —— **会话隔离** 与 HELD 合并的回归测试。
 *
 * 覆盖两个真实缺陷形态（都曾真实存在于上线路径上）：
 *
 * 1. 消息只按 `fleetId` 索引 ⇒ DM 与群聊共用一条时间线。后端 `fleet_messages`
 *    的会话列沦为**只写不读**字段；更严重的是 DM 内容会漏进群聊的路由 prompt。
 * 2. `DispatchEvent::Held.maxSeq` **零消费端**、`heldMessages` 也未并入时间线 ——
 *    服务端「展示」了未读消息，前端却没让用户看到，契约名存实亡。
 *
 * 断言都带**区分力**：若退回「按 fleetId 索引」或「覆盖式加载」，本文件必红。
 */

import { beforeEach, describe, expect, it, vi } from "vitest";

const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }));

vi.mock("@/lib/invoke", () => ({
  invoke: invokeMock,
  listen: vi.fn(() => Promise.resolve(() => {})),
  // 浏览器模式分支：store 会自建 mockChannel 并把 onEvent 传给 invoke
  isTauri: () => false,
  logIpcError: vi.fn(() => vi.fn()),
}));

vi.mock("@/lib/errorI18n", () => ({
  translateBackendError: (e: unknown) => String(e),
}));

import { useOfficeStore } from "@/stores/feature/officeStore";
import type { DispatchEvent, FleetMessage } from "@/types";
import { CONVERSATION_GROUP, conversationDm, conversationKey } from "@/types";

const FLEET = "fleet-1";
const DM_CONVO = conversationDm("risk");
const GROUP_KEY = conversationKey(FLEET, CONVERSATION_GROUP);
const DM_KEY = conversationKey(FLEET, DM_CONVO);

function msg(seq: number, conversationId: string, content?: string): FleetMessage {
  return {
    id: `${conversationId}-${seq}`,
    fleetId: FLEET,
    conversationId,
    seq,
    authorKind: "agent",
    authorId: "agent-1",
    authorSlug: "risk",
    authorDisplayName: "Risk",
    content: content ?? `msg-${seq}`,
    createdAt: 1735689600000 + seq,
  };
}

/** 取某会话时间线的 seq 列表（断言用，避免依赖对象身份） */
function seqsOf(key: string): number[] {
  return (useOfficeStore.getState().messagesByConversation[key] ?? []).map((m) => m.seq);
}

/** 记录 invoke 收到的 (cmd, args)，供「收尾重读用了哪个会话」这类断言使用 */
let calls: Array<{ cmd: string; args: Record<string, unknown> }> = [];

beforeEach(() => {
  calls = [];
  vi.clearAllMocks();
  useOfficeStore.setState({
    messagesByConversation: {},
    dispatchEvents: [],
    error: null,
    loading: false,
  });
});

describe("officeStore — 会话隔离", () => {
  it("加载会话 B 不得覆盖会话 A 的时间线（键是 fleetId::conversationId）", async () => {
    invokeMock.mockImplementation(async (cmd: string, args: Record<string, unknown>) => {
      calls.push({ cmd, args });
      if (cmd === "fleet_list_messages") {
        return args.conversationId === DM_CONVO
          ? [msg(1, DM_CONVO, "dm-1")]
          : [msg(1, CONVERSATION_GROUP, "group-1")];
      }
      return undefined;
    });

    await useOfficeStore.getState().loadMessages(FLEET, CONVERSATION_GROUP);
    expect(seqsOf(GROUP_KEY)).toEqual([1]);

    await useOfficeStore.getState().loadMessages(FLEET, DM_CONVO);

    // ★ 区分力断言：旧实现按 fleetId 索引，DM 的加载会把群聊内容顶掉，
    //   于是这里 group 会变成 ["dm-1"]（或直接消失）。
    expect((useOfficeStore.getState().messagesByConversation[GROUP_KEY] ?? []).map((m) => m.content))
      .toEqual(["group-1"]);
    expect((useOfficeStore.getState().messagesByConversation[DM_KEY] ?? []).map((m) => m.content))
      .toEqual(["dm-1"]);
  });

  it("loadMessages 必须把 conversationId 传给后端（否则后端只能按舰队过滤）", async () => {
    invokeMock.mockImplementation(async (cmd: string, args: Record<string, unknown>) => {
      calls.push({ cmd, args });
      return [];
    });

    await useOfficeStore.getState().loadMessages(FLEET, DM_CONVO);

    const call = calls.find((c) => c.cmd === "fleet_list_messages");
    expect(call?.args.conversationId).toBe(DM_CONVO);
    expect(call?.args.fleetId).toBe(FLEET);
  });

  it("directMessage 落在该成员的私信会话，而非群聊", async () => {
    invokeMock.mockImplementation(async (cmd: string, args: Record<string, unknown>) => {
      calls.push({ cmd, args });
      return undefined;
    });

    await useOfficeStore.getState().directMessage({
      fleetId: FLEET,
      agentSlug: "risk",
      userMessage: "评估一下当前风险",
    });

    const reload = calls.find((c) => c.cmd === "fleet_list_messages");
    expect(reload?.args.conversationId).toBe(DM_CONVO);
    // 未触碰群聊会话
    expect(seqsOf(GROUP_KEY)).toEqual([]);
  });
});

describe("officeStore — HELD（展示即已见）", () => {
  it("heldMessages 立刻并入时间线，且收尾重读的较短窗口不得把它们清掉", async () => {
    invokeMock.mockImplementation(async (cmd: string, args: Record<string, unknown>) => {
      calls.push({ cmd, args });
      if (cmd === "fleet_dispatch") {
        const onEvent = args.onEvent as { onmessage: (e: DispatchEvent) => void };
        const held: DispatchEvent = {
          type: "held",
          maxSeq: 7,
          heldMessages: [msg(6, CONVERSATION_GROUP), msg(7, CONVERSATION_GROUP)],
        };
        onEvent.onmessage(held);
        // 重复投递同一条事件（重连/重放）：必须被 seq 去重
        onEvent.onmessage(held);
        return undefined;
      }
      if (cmd === "fleet_list_messages") {
        // 模拟服务端只返回**较短窗口**（只有最新一条）——
        // 若 store 用覆盖式加载，seq=6 会被清掉。
        return [msg(7, CONVERSATION_GROUP)];
      }
      return undefined;
    });

    await useOfficeStore.getState().dispatch({ fleetId: FLEET, userMessage: "hi" });

    expect(seqsOf(GROUP_KEY)).toEqual([6, 7]);
  });

  it("heldMessages 不得污染其它会话，且 maxSeq 随事件保留可供 UI 展示", async () => {
    let seen: DispatchEvent | undefined;
    invokeMock.mockImplementation(async (cmd: string, args: Record<string, unknown>) => {
      calls.push({ cmd, args });
      if (cmd === "fleet_dispatch") {
        const onEvent = args.onEvent as { onmessage: (e: DispatchEvent) => void };
        seen = {
          type: "held",
          maxSeq: 42,
          heldMessages: [msg(41, CONVERSATION_GROUP)],
        };
        onEvent.onmessage(seen);
        return undefined;
      }
      return [];
    });

    await useOfficeStore.getState().dispatch({ fleetId: FLEET, userMessage: "hi" });

    expect(useOfficeStore.getState().messagesByConversation[DM_KEY]).toBeUndefined();
    // maxSeq 是**会话水位**，被 UI 用于提示「已推进到 #N」
    expect(seen?.type === "held" && seen.maxSeq).toBe(42);
    expect(useOfficeStore.getState().dispatchEvents.some((e) => e.type === "held")).toBe(true);
  });
});
