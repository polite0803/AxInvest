// SPDX-License-Identifier: AGPL-3.0-only

/**
 * 卡片贡献注册表（任务 #216 前置 ①）的验收用例。
 *
 * 打在**宿主键与归属**上，不回头自证判据函数：
 * 1. 键只由宿主拼（`${pluginId}:${id}`）、`schemaId` 由宿主按 `plugin.{pluginId}.{key}` 拼；
 * 2. 只挂 `enabled` 的插件、`surface` 表外值丢弃、同键先到先得（不覆盖）；
 * 3. 旧键 `schemaId` 能反查到归属（聊天侧迁移的判据）；
 * 4. `ensureCardContributionsLoaded` 幂等（一次会话只发一次 IPC），失败按「没有插件卡」。
 */

import type { PluginTaskCardsInfoDto } from "@/types";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/invoke", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/lib/invoke")>();
  return { ...actual, invoke: vi.fn() };
});

import { invoke } from "@/lib/invoke";
import {
  __resetCardRegistryForTests,
  cardKey,
  ensureCardContributionsLoaded,
  getContributionByKey,
  getContributionBySchemaId,
  getContributions,
  registerCardContributions,
} from "../pluginCardRegistry";

const invokeMock = vi.mocked(invoke);

function info(overrides: Partial<PluginTaskCardsInfoDto> = {}): PluginTaskCardsInfoDto {
  return {
    id: "demo",
    name: "Demo",
    version: "1.0.0",
    description: "demo plugin",
    author: null,
    enabled: true,
    taskCards: [{
      id: "release-note",
      title: "发行说明",
      surface: "chat",
      summaryCommand: "rows",
      detailSchema: "release-note-card",
      tone: "gold",
      order: 10,
      actions: [{ id: "open", title: "打开", command: "rows", mutating: false }],
    }],
    ...overrides,
  };
}

beforeEach(() => {
  __resetCardRegistryForTests();
  invokeMock.mockReset();
});

describe("pluginCardRegistry —— 键、归属与占位规则", () => {
  it("键 = `${pluginId}:${id}`，schemaId 由宿主按 plugin.{pluginId}.{key} 拼", () => {
    expect(registerCardContributions([info()])).toBe(1);

    const contribution = getContributionByKey(cardKey("demo", "release-note"));
    expect(contribution).toBeDefined();
    expect(contribution?.pluginId).toBe("demo");
    expect(contribution?.surface).toBe("chat");
    expect(contribution?.detailSchema).toBe("release-note-card");
    expect(contribution?.schemaId).toBe("plugin.demo.release-note-card");
    // 旧键迁移入口：按卡体行 id 反查到同一条贡献（归属由注册表证明）
    expect(getContributionBySchemaId("plugin.demo.release-note-card")).toBe(contribution);
  });

  it("只挂 enabled 的插件；surface 表外值丢弃（不猜落位）", () => {
    expect(registerCardContributions([info({ id: "off", enabled: false })])).toBe(0);
    expect(getContributions("chat")).toHaveLength(0);

    const bad = info({ id: "bad" });
    bad.taskCards = [{ ...bad.taskCards[0], surface: "overview" as never }];
    expect(registerCardContributions([bad])).toBe(0);
    expect(getContributionByKey(cardKey("bad", "release-note"))).toBeUndefined();
  });

  it("同键先到先得：后注册的不覆盖（宿主优先的落点）", () => {
    expect(registerCardContributions([info()])).toBe(1);
    const hijack = info();
    hijack.taskCards = [{ ...hijack.taskCards[0], title: "被抢的标题" }];

    expect(registerCardContributions([hijack])).toBe(0);
    expect(getContributionByKey(cardKey("demo", "release-note"))?.title).toBe("发行说明");
  });

  it("按 surface 分桶并按 order 升序（同值保持注册顺序）", () => {
    const infos = info();
    infos.taskCards = [
      { ...infos.taskCards[0], id: "later", order: 90, detailSchema: null },
      { ...infos.taskCards[0], id: "earlier", order: 10 },
      { ...infos.taskCards[0], id: "card-face", order: 50, surface: "task_card" },
    ];
    expect(registerCardContributions([infos])).toBe(3);

    expect(getContributions("chat").map((c) => c.id)).toEqual(["earlier", "later"]);
    expect(getContributions("task_card").map((c) => c.id)).toEqual(["card-face"]);
    // 没有 detailSchema ⇒ 纯计数卡：没有卡体行 id，聊天侧不应画它
    expect(getContributions("chat")[1].schemaId).toBeUndefined();
  });

  it("投影加载幂等：每路投影一次会话只发一次 IPC，失败按「没有卡」处理", async () => {
    invokeMock.mockRejectedValue(new Error("PLUGIN_LIST_TASK_CARDS_FAILED"));

    expect(await ensureCardContributionsLoaded()).toBe(0);
    expect(await ensureCardContributionsLoaded()).toBe(0);
    // 两路投影（宿主 + 插件）各一次；幂等缓存让重复 ensure 不追加 IPC
    expect(invokeMock).toHaveBeenCalledTimes(2);
    expect(invokeMock).toHaveBeenCalledWith("task_card_host_cards");
    expect(invokeMock).toHaveBeenCalledWith("plugin_list_task_cards");
    expect(getContributions("chat")).toHaveLength(0);
    expect(getContributions("task_card")).toHaveLength(0);
  });
});
