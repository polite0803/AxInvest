// SPDX-License-Identifier: AGPL-3.0-only

/**
 * 浏览器模式 Bot 数据面回归锁（2026-11-22）。
 *
 * ## 钉住的两个缺陷
 *
 * ① **Web 模式线程过滤恒空**：`create_conversation` / `update_conversation`
 *    此前完全不认识 `botId`，落库对象没有归属键 ⇒ 前端按 Bot 过滤永远得到空列表。
 *    本文件第一条用例即「建会话时传 botId，list_conversations 必须带回 botId」。
 *
 * ② **e2e 假绿**：CI 曾用标题正则跳过用例。数据面的用例不依赖 UI，先在单测层
 *    锁住 mock 契约，e2e 才有可信的地基（e2e/bots.spec.ts 用同一份 localStorage 形状）。
 *
 * ## 为什么用 `handleCommand` 而不是直接调 `handleBotCommand`
 *
 * 前端真实入口是 `@/lib/invoke` → `handleCommand`：入参会递归 camel→snake，出参
 * 递归 snake→camel。直接调内部函数会绕过这两次转换，测出来的键形与线上不一致。
 */

import { beforeEach, describe, expect, it } from "vitest";

import { handleCommand } from "../browserMock";

interface BotDto {
  id: string;
  name: string;
  soul: string;
  soulHash: string;
  approvalMode: string;
  isEnabled: boolean;
  title: string | null;
}

interface ConvDto {
  id: string;
  title: string;
  botId?: string | null;
  isArchived: boolean;
}

async function makeBot(name = "投研助手", soul = "# 只回答 A 股问题"): Promise<BotDto> {
  return handleCommand<BotDto>("create_bot", { input: { name, soul } });
}

interface RoutineDto {
  id: string;
  name: string;
  description: string;
  taskType: string;
  cronExpression: string | null;
  status: "active" | "paused" | "disabled";
  botId: string;
  timezone: string | null;
  nextRunAt: string | null;
  createdAt: string;
}

async function makeRoutine(botId: string, name = "例程"): Promise<RoutineDto> {
  return handleCommand<RoutineDto>("create_bot_scheduled_task", {
    botId,
    name,
    prompt: name + " 提示词",
    cronExpression: "30 8 * * 1-5",
  });
}

async function makeConversation(botId?: string | null, title = "线程"): Promise<ConvDto> {
  return handleCommand<ConvDto>("create_conversation", {
    title,
    modelId: "mock-model",
    providerId: "mock-provider",
    ...(botId === undefined ? {} : { botId }),
  });
}

describe("browserMock: Bot 数据面", () => {
  beforeEach(() => {
    localStorage.clear();
  });

  it("缺陷①回归：create_conversation 传 botId 后，list_conversations 必须带回 botId", async () => {
    const bot = await makeBot();
    const conv = await makeConversation(bot.id, "归属线程");
    expect(conv.botId).toBe(bot.id);

    const all = await handleCommand<ConvDto[]>("list_conversations");
    expect(all.map((c) => c.botId)).toContain(bot.id);
    // 过滤口径与前端一致：没有归属键的会话不得混进来。
    expect(all.filter((c) => c.botId === bot.id)).toHaveLength(1);
  });

  it("update_conversation 可补写 botId，也可用 null 显式解绑", async () => {
    const bot = await makeBot();
    const conv = await makeConversation(undefined, "先建后归属");
    expect(conv.botId ?? null).toBeNull();

    const assigned = await handleCommand<ConvDto>("update_conversation", {
      id: conv.id,
      input: { botId: bot.id },
    });
    expect(assigned.botId).toBe(bot.id);

    const detached = await handleCommand<ConvDto>("update_conversation", {
      id: conv.id,
      input: { botId: null },
    });
    expect(detached.botId ?? null).toBeNull();
  });

  it("list_bot_threads 只返回该 Bot 的线程（排除未归属与其它 Bot），并支持归档开关", async () => {
    const a = await makeBot("A");
    const b = await makeBot("B");
    const aThread = await makeConversation(a.id, "A-线程");
    const bThread = await makeConversation(b.id, "B-线程");
    const orphan = await makeConversation(undefined, "未归属");

    const threads = await handleCommand<ConvDto[]>("list_bot_threads", { botId: a.id });
    expect(threads.map((t) => t.id)).toEqual([aThread.id]);
    expect(threads.map((t) => t.id)).not.toContain(bThread.id);
    expect(threads.map((t) => t.id)).not.toContain(orphan.id);

    // 归档线程默认不返回
    const convs = JSON.parse(localStorage.getItem("axagent_conversations") ?? "[]") as Record<
      string,
      unknown
    >[];
    const target = convs.find((c) => c.id === aThread.id);
    if (target) {
      target.is_archived = true;
    }
    localStorage.setItem("axagent_conversations", JSON.stringify(convs));
    expect(await handleCommand<ConvDto[]>("list_bot_threads", { botId: a.id })).toHaveLength(0);
    expect(
      await handleCommand<ConvDto[]>("list_bot_threads", {
        botId: a.id,
        includeArchived: true,
      }),
    ).toHaveLength(1);
  });

  it("create_bot / list_bots / get_bot 返回完整 camelCase DTO", async () => {
    const bot = await makeBot("投研助手", "# SOUL");
    expect(bot.id).toMatch(/\S/);
    expect(bot.name).toBe("投研助手");
    expect(bot.soul).toBe("# SOUL");
    expect(bot.soulHash).toMatch(/^[0-9a-f]{8}$/);
    expect(bot.approvalMode).toBe("full");
    expect(bot.isEnabled).toBe(true);

    const list = await handleCommand<BotDto[]>("list_bots");
    expect(list.map((b) => b.id)).toEqual([bot.id]);
    const got = await handleCommand<BotDto>("get_bot", { botId: bot.id });
    expect(got.id).toBe(bot.id);

    // 禁用后默认列表不返回，includeDisabled 才返回
    await handleCommand("update_bot", { botId: bot.id, input: { isEnabled: false } });
    expect(await handleCommand<BotDto[]>("list_bots")).toHaveLength(0);
    expect(await handleCommand<BotDto[]>("list_bots", { includeDisabled: true })).toHaveLength(1);
  });

  it("delete_bot 保留线程并解绑（ADR-009）", async () => {
    const bot = await makeBot();
    await makeConversation(bot.id, "要保留的线程");

    const res = await handleCommand<{ detachedThreads: number; memoryPath: string }>("delete_bot", {
      botId: bot.id,
    });
    expect(res.detachedThreads).toBe(1);
    expect(res.memoryPath).toContain(bot.id);
    expect(await handleCommand<BotDto[]>("list_bots")).toHaveLength(0);

    const all = await handleCommand<ConvDto[]>("list_conversations");
    expect(all).toHaveLength(1);
    expect(all[0].botId ?? null).toBeNull();
  });

  it("read_bot_soul / write_bot_soul 的 drift 判据与后端一致", async () => {
    const bot = await makeBot("S", "# v1");
    const initial = await handleCommand<{ content: string; drift: boolean }>("read_bot_soul", {
      botId: bot.id,
    });
    expect(initial.drift).toBe(false);

    const written = await handleCommand<{ content: string; hash: string; drift: boolean }>(
      "write_bot_soul",
      { botId: bot.id, content: "# v2" },
    );
    expect(written.content).toBe("# v2");
    expect(written.drift).toBe(false);

    // 模拟用户脱离 App 改了镜像文件（localStorage 里的镜像副本）
    const mirrors = JSON.parse(localStorage.getItem("axagent_bot_soul_mirrors") ?? "{}") as Record<
      string,
      string
    >;
    mirrors[bot.id] = "# 外部手改";
    localStorage.setItem("axagent_bot_soul_mirrors", JSON.stringify(mirrors));

    const drifted = await handleCommand<{ content: string; drift: boolean }>("read_bot_soul", {
      botId: bot.id,
    });
    expect(drifted.drift).toBe(true);
    // 真相源是 DB：内容仍是写库值，不自动被外部文件覆盖
    expect(drifted.content).toBe("# v2");
  });

  it("read_bot_memory 返回 200 行 / 24 KB 预算与主题清单", async () => {
    const bot = await makeBot("M");
    localStorage.setItem(
      `axagent_bot_memory_${bot.id}`,
      JSON.stringify({
        indexContent: "# MEMORY\n- 偏好：先看财报",
        topics: [{ relativePath: "memory/invest.md", category: "invest", title: "投研", updatedAt: 1 }],
      }),
    );
    const mem = await handleCommand<{
      indexContent: string;
      lineCount: number;
      byteCount: number;
      budget: { maxLines: number; maxBytes: number };
      topics: { relativePath: string }[];
    }>("read_bot_memory", { botId: bot.id });
    expect(mem.indexContent).toContain("偏好");
    expect(mem.lineCount).toBe(2);
    expect(mem.byteCount).toBe(new TextEncoder().encode(mem.indexContent).length);
    expect(mem.budget).toEqual({ maxLines: 200, maxBytes: 24 * 1024 });
    expect(mem.topics.map((t) => t.relativePath)).toEqual(["memory/invest.md"]);
  });

  // ── P2-a 例程（IPC-P2 §2 / browserMockBots 例程命令）──────────────────

  it("P2-a: create/list 例程走冻结契约，DTO 带 botId/timezone/taskType", async () => {
    const bot = await makeBot("R");
    const task = await handleCommand<RoutineDto>("create_bot_scheduled_task", {
      botId: bot.id,
      name: "每日复盘",
      prompt: "总结今日行情",
      cronExpression: "30 8 * * 1-5",
      timezone: "Asia/Shanghai",
    });
    expect(task.taskType).toBe("bot_routine");
    expect(task.botId).toBe(bot.id);
    expect(task.timezone).toBe("Asia/Shanghai");
    expect(task.status).toBe("active");
    expect(task.cronExpression).toBe("30 8 * * 1-5");
    expect(task.nextRunAt).toBeTruthy();

    const list = await handleCommand<RoutineDto[]>("list_bot_scheduled_tasks", { botId: bot.id });
    expect(list).toHaveLength(1);
    expect(list[0].id).toBe(task.id);
  });

  it("P2-a: 校验链与后端同码（空 prompt / 坏 cron / 坏时区 / Bot 不存在）", async () => {
    const bot = await makeBot("V");
    await expect(
      handleCommand("create_bot_scheduled_task", {
        botId: bot.id, name: "x", prompt: "   ", cronExpression: "30 8 * * 1-5",
      }),
    ).rejects.toThrow(/COMMON_INVALID_INPUT/);
    await expect(
      handleCommand("create_bot_scheduled_task", {
        botId: bot.id, name: "x", prompt: "p", cronExpression: "每天早上",
      }),
    ).rejects.toThrow(/BOT_SCHEDULE_CRON_INVALID/);
    await expect(
      handleCommand("create_bot_scheduled_task", {
        botId: bot.id, name: "x", prompt: "p", cronExpression: "30 8 * * 1-5", timezone: "UTC+8",
      }),
    ).rejects.toThrow(/BOT_SCHEDULE_TIMEZONE_INVALID/);
    await expect(
      handleCommand("create_bot_scheduled_task", {
        botId: "missing-bot", name: "x", prompt: "p", cronExpression: "30 8 * * 1-5",
      }),
    ).rejects.toThrow(/BOT_NOT_FOUND/);
    await expect(handleCommand("list_bot_scheduled_tasks", { botId: "missing-bot" })).rejects.toThrow(
      /BOT_NOT_FOUND/,
    );
  });

  it("P2-a: pause/resume/update/delete 与删除级联（GAP G-10）", async () => {
    const bot = await makeBot("C");
    const other = await makeBot("O");
    await makeRoutine(bot.id, "A 例程");
    await makeRoutine(bot.id, "B 例程");
    await makeRoutine(other.id, "O 例程");
    // 顺手造一条非 Bot 普通任务，验证 bot 分支不拦截普通任务
    localStorage.setItem(
      "axagent_bot_routines",
      JSON.stringify(
        (JSON.parse(localStorage.getItem("axagent_bot_routines") ?? "[]") as RoutineDto[]).concat([
          { ...(JSON.parse(localStorage.getItem("axagent_bot_routines") ?? "[]") as RoutineDto[])[0], id: "plain-task", taskType: "backup", botId: null as unknown as string },
        ]),
      ),
    );

    const list = await handleCommand<RoutineDto[]>("list_bot_scheduled_tasks", { botId: bot.id });
    const a = list.find((t) => t.name === "A 例程")!;
    expect(list).toHaveLength(2);

    await handleCommand("pause_scheduled_task", { taskId: a.id });
    let after = await handleCommand<RoutineDto[]>("list_bot_scheduled_tasks", { botId: bot.id });
    expect(after.find((t) => t.id === a.id)!.status).toBe("paused");
    await handleCommand("resume_scheduled_task", { taskId: a.id });
    after = await handleCommand<RoutineDto[]>("list_bot_scheduled_tasks", { botId: bot.id });
    expect(after.find((t) => t.id === a.id)!.status).toBe("active");

    // §2.5 编辑：完整 DTO + 顶层 timezone（null=清除回落 Local）
    const updated = { ...a, name: "A2 例程", description: "新提示词", cronExpression: "0 10 * * 1" };
    await handleCommand("update_scheduled_task", { taskId: a.id, task: updated, timezone: null });
    after = await handleCommand<RoutineDto[]>("list_bot_scheduled_tasks", { botId: bot.id });
    const a2 = after.find((t) => t.id === a.id)!;
    expect(a2.name).toBe("A2 例程");
    expect(a2.description).toBe("新提示词");
    expect(a2.cronExpression).toBe("0 10 * * 1");
    expect(a2.timezone ?? null).toBeNull();

    // 普通任务不被 bot 分支拦截：delete_scheduled_task 走 browserMock 原分支返回 undefined
    await expect(handleCommand("delete_scheduled_task", { taskId: "plain-task" })).resolves.toBeUndefined();

    // 删除 Bot 级联只删本 Bot 例程（GAP G-10）
    const res = await handleCommand<{ detachedThreads: number; removedRoutines: number }>(
      "delete_bot", { botId: bot.id },
    );
    expect(res.removedRoutines).toBe(2);
    expect(await handleCommand<RoutineDto[]>("list_bot_scheduled_tasks", { botId: other.id })).toHaveLength(1);
    // Bot 没了：list 报 BOT_NOT_FOUND（级联后无例程可删但仍校验存在性）
    await expect(handleCommand("list_bot_scheduled_tasks", { botId: bot.id })).rejects.toThrow(/BOT_NOT_FOUND/);
  });

  it("校验失败返回结构化错误码（与后端 ErrorResponse 同形）", async () => {
    await expect(handleCommand("create_bot", { input: { name: "   " } })).rejects.toThrow(
      /BOT_NAME_INVALID/,
    );
    await expect(
      handleCommand("create_bot", { input: { name: "x", approvalMode: "yolo" } }),
    ).rejects.toThrow(/BOT_APPROVAL_MODE_INVALID/);
    await expect(
      handleCommand("create_bot", { input: { name: "x", modelId: "m", providerId: null } }),
    ).rejects.toThrow(/BOT_MODEL_PAIR_INVALID/);
    await expect(handleCommand("read_bot_soul", { botId: "missing" })).rejects.toThrow(
      /BOT_NOT_FOUND/,
    );
  });
});
