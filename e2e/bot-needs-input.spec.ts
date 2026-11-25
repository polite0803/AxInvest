// SPDX-License-Identifier: AGPL-3.0-only

/**
 * P4-b 持久化待答请求 e2e（PLAN-P4 §6.4 B5 场景）。
 *
 * 分类边界与 bot-quota.spec.ts 同口径：
 *
 * A. 契约自检（现在可跑）：验证 shim 按 IPC-P4 §1/§3/§4 + ADR-P4 D5/D6/D7 实现的
 *    list/answer/cancel 三命令、agent_respond_ask 的 P4 分支与 `bot-input-request-updated`
 *    事件语义（pending→answered/expired/cancelled 各一次、错误码、过滤与排序、300s 过期）。
 *    Rust 侧生命周期（落库/投递 oneshot/重启清扫）由 `src-tauri/tests/bot_input.rs` 锁定。
 *
 * B. 页面证据（B4 未落地：`test.fixme` 显式待办）：
 *    Bot 线程持久化卡片、刷新恢复、过期/取消终态、非 Bot 聊天零变化、侧栏待答数。
 *    点亮条件：B4 落盘且下列 data-testid 存在，随后把 `test.fixme` 改 `test`：
 *      bot-input-request-card、bot-input-request-question、bot-input-request-options、
 *      bot-input-request-answer、bot-input-request-submit、bot-input-request-cancel、
 *      bot-input-request-status、bot-input-badge。
 *
 * 未在本文件覆盖：「重启清扫 → pending 全过期」需要真实进程重启与 SQLite，
 * 浏览器面无法制造；该断言落在 `src-tauri/tests/bot_input.rs`（B2 expire_stale 用例）。
 */

import { expect, test, type Page } from "@playwright/test";
import { installP3Shim } from "./helpers/p3-tauri-shim";

const BASE_BOT = {
  title: null, description: null, soulHash: "00000000", soulDrift: false, icon: "🤖",
  color: null, section: "default", modelId: null, providerId: null, effort: null,
  workdir: null, agentProfileId: null, approvalMode: "full", isEnabled: true, sortOrder: 0,
  createdAt: 1_700_000_000_000, updatedAt: 1_700_000_000_000,
};

const INPUT_BOTS = [{ ...BASE_BOT, id: "bot-input-a", name: "问答助手" }];
const TTL_MS = 300_000; // ADR-P4 D5

async function bootShim(page: Page) {
  await page.addInitScript(installP3Shim, { bots: INPUT_BOTS, departments: [] });
  await page.goto("/bots");
  await page.waitForFunction(() => Boolean((window as any).__P3_TEST__));
}

type IpcResult = { ok: true; value: any } | { ok: false; error: string };

async function ipc(page: Page, cmd: string, args: any): Promise<IpcResult> {
  return page.evaluate(async ([c, a]) => {
    const t = (window as any).__P3_TEST__;
    try {
      return { ok: true as const, value: await t.invoke(c, a) };
    } catch (e: any) {
      return { ok: false as const, error: String(e?.message ?? e) };
    }
  }, [cmd, args] as [string, any]);
}

/** 捕获 bot-input-request-updated —— 证明「每个状态转各发一次」，不靠 UI 推断。 */
async function captureEvents(page: Page) {
  await page.evaluate(async () => {
    const w = window as any;
    w.__P4_EVENTS__ = [] as any[];
    const handler = w.__TAURI_INTERNALS__.transformCallback(
      (e: any) => w.__P4_EVENTS__.push(e.payload),
    );
    await w.__P3_TEST__.invoke("plugin:event|listen", {
      event: "bot-input-request-updated", target: { kind: "Any" }, handler,
    });
  });
}

async function events(page: Page): Promise<any[]> {
  return page.evaluate(() => (window as any).__P4_EVENTS__);
}

const pendingSeed = (over: Record<string, any> = {}) => ({
  askId: "ask-1", conversationId: "conv-bot-1", botId: "bot-input-a",
  origin: "person", question: "请确认标的池范围？", options: ["沪深300", "中证500"],
  ...over,
});

/** 不带参数 = 默认 pending 行；传 over 覆盖字段（已含默认值，见 pendingSeed）。 */
async function seedPending(page: Page, over: Record<string, any> = {}): Promise<any> {
  return page.evaluate((o) => (window as any).__P3_TEST__.seedInputRequest(o), pendingSeed(over));
}

// ── A. 契约自检（现在可跑） ─────────────────────────────────────────────

test.describe("P4-b 待答请求 · IPC 契约自检（shim）", () => {
  test("AC-B5-1：pending 行落库 → list 命中 + 300s 过期 + pending 事件一次", async ({ page }) => {
    await bootShim(page);
    await captureEvents(page);

    const row = await seedPending(page);
    expect(row).toMatchObject({
      askId: "ask-1", conversationId: "conv-bot-1", botId: "bot-input-a",
      workflowRunId: null, assignmentId: null, origin: "person",
      question: "请确认标的池范围？", options: ["沪深300", "中证500"],
      status: "pending", answer: null, answeredAt: null,
    });
    // MODEL-P4：expires_at = created_at + 300_000（ADR-P4 D5）
    expect(row.expiresAt - row.createdAt).toBe(TTL_MS);

    const listed = await ipc(page, "list_bot_input_requests", {
      filter: { conversationId: "conv-bot-1" },
    });
    expect(listed.ok).toBe(true);
    expect((listed as any).value).toHaveLength(1);
    expect((listed as any).value[0].id).toBe(row.id);

    const evs = await events(page);
    expect(evs).toHaveLength(1);
    expect(evs[0]).toMatchObject({ id: row.id, status: "pending" });
  });

  test("AC-B5-2：作答 → answered + 事件 + 答案落库；重复作答 → NOT_PENDING", async ({ page }) => {
    await bootShim(page);
    await captureEvents(page);
    const row = await seedPending(page);

    const answered = await ipc(page, "answer_bot_input_request", {
      requestId: row.id, answer: "沪深300",
    });
    expect(answered.ok).toBe(true);
    expect((answered as any).value).toMatchObject({ status: "answered", answer: "沪深300" });
    expect((answered as any).value.answeredAt).toBeGreaterThan(0);

    const again = await ipc(page, "answer_bot_input_request", {
      requestId: row.id, answer: "中证500",
    });
    expect(again).toEqual({ ok: false, error: "BOT_INPUT_REQUEST_NOT_PENDING" });

    const evs = await events(page);
    expect(evs.map((e) => e.status)).toEqual(["pending", "answered"]);
    // 重复作答不得再发事件（事件次数 = 状态转换次数）
    expect(evs).toHaveLength(2);
  });

  test("AC-B5-3：去空白后为空 → BOT_INPUT_ANSWER_EMPTY，且仍为 pending", async ({ page }) => {
    await bootShim(page);
    const row = await seedPending(page);

    for (const answer of ["", "   ", "\n\t "]) {
      const res = await ipc(page, "answer_bot_input_request", { requestId: row.id, answer });
      expect(res, JSON.stringify(answer)).toEqual({ ok: false, error: "BOT_INPUT_ANSWER_EMPTY" });
    }
    const listed = await ipc(page, "list_bot_input_requests", { filter: { status: "pending" } });
    expect((listed as any).value).toHaveLength(1);
  });

  test("ACK-B5-4：答复/取消不存在 id → BOT_INPUT_REQUEST_NOT_FOUND", async ({ page }) => {
    await bootShim(page);
    expect(await ipc(page, "answer_bot_input_request", { requestId: "input-ghost", answer: "x" }))
      .toEqual({ ok: false, error: "BOT_INPUT_REQUEST_NOT_FOUND" });
    expect(await ipc(page, "cancel_bot_input_request", { requestId: "input-ghost" }))
      .toEqual({ ok: false, error: "BOT_INPUT_REQUEST_NOT_FOUND" });
  });

  test("AC-B5-5：cancel → cancelled + 事件；取消后再作答 → NOT_PENDING", async ({ page }) => {
    await bootShim(page);
    await captureEvents(page);
    const row = await seedPending(page);

    const cancelled = await ipc(page, "cancel_bot_input_request", { requestId: row.id });
    expect(cancelled.ok).toBe(true);
    expect((cancelled as any).value.status).toBe("cancelled");

    expect(await ipc(page, "answer_bot_input_request", { requestId: row.id, answer: "沪深300" }))
      .toEqual({ ok: false, error: "BOT_INPUT_REQUEST_NOT_PENDING" });
    expect(await ipc(page, "cancel_bot_input_request", { requestId: row.id }))
      .toEqual({ ok: false, error: "BOT_INPUT_REQUEST_NOT_PENDING" });

    expect((await events(page)).map((e) => e.status)).toEqual(["pending", "cancelled"]);
  });

  test("AC-B5-6：过期清扫只命中已到期 pending（300s），各发一次 expired 事件", async ({ page }) => {
    await bootShim(page);
    await captureEvents(page);

    const at = Date.now();
    const stale = await seedPending(page, pendingSeed({
      askId: "ask-stale", conversationId: "conv-bot-1",
      createdAt: at - TTL_MS - 1_000, expiresAt: at - 1_000,
    }));
    const fresh = await seedPending(page, pendingSeed({
      askId: "ask-fresh", conversationId: "conv-bot-1", createdAt: at, expiresAt: at + TTL_MS,
    }));

    const expired = await page.evaluate(() => (window as any).__P3_TEST__.expireInputRequests());
    expect(expired.map((r: any) => r.id)).toEqual([stale.id]);

    const listed = await ipc(page, "list_bot_input_requests", {
      filter: { conversationId: "conv-bot-1" },
    });
    const byId = Object.fromEntries(((listed as any).value as any[]).map((r) => [r.id, r.status]));
    expect(byId[stale.id]).toBe("expired");
    expect(byId[fresh.id]).toBe("pending");

    const evs = await events(page);
    expect(evs.filter((e) => e.status === "expired").map((e) => e.id)).toEqual([stale.id]);
    expect(evs).toHaveLength(3); // pending×2 + expired×1

    // 过期后再作答必须被拒（D7：只答给活着的 pending）
    expect(await ipc(page, "answer_bot_input_request", { requestId: stale.id, answer: "x" }))
      .toEqual({ ok: false, error: "BOT_INPUT_REQUEST_NOT_PENDING" });
  });

  test("命令 list：四过滤组合 + status 非法 → ERR_VALIDATION + created_at DESC", async ({ page }) => {
    await bootShim(page);
    const at = Date.now();
    await seedPending(page, pendingSeed({ askId: "a-old", createdAt: at - 2_000, expiresAt: at + TTL_MS }));
    await seedPending(page, pendingSeed({
      askId: "a-new", conversationId: "conv-bot-2", createdAt: at - 1_000, expiresAt: at + TTL_MS,
    }));
    await seedPending(page, pendingSeed({
      askId: "a-run", conversationId: "conv-bot-1", origin: "workflow",
      workflowRunId: "run-1", createdAt: at, expiresAt: at + TTL_MS,
    }));

    const all = await ipc(page, "list_bot_input_requests", { filter: {} });
    expect(((all as any).value as any[]).map((r) => r.askId)).toEqual(["a-run", "a-new", "a-old"]);

    const byConv = await ipc(page, "list_bot_input_requests", { filter: { conversationId: "conv-bot-1" } });
    expect(((byConv as any).value as any[]).map((r) => r.askId)).toEqual(["a-run", "a-old"]);

    const byBot = await ipc(page, "list_bot_input_requests", { filter: { botId: "bot-input-a" } });
    expect((byBot as any).value).toHaveLength(3);

    const byRun = await ipc(page, "list_bot_input_requests", { filter: { workflowRunId: "run-1" } });
    expect(((byRun as any).value as any[]).map((r) => r.askId)).toEqual(["a-run"]);

    const byStatus = await ipc(page, "list_bot_input_requests", { filter: { status: "pending" } });
    expect((byStatus as any).value).toHaveLength(3);

    expect(await ipc(page, "list_bot_input_requests", { filter: { status: "bogus" } }))
      .toEqual({ ok: false, error: "ERR_VALIDATION" });
  });

  test("agent_respond_ask：命中 askId → answered；未命中 → 保持 P2 行为（undefined，不新增行）", async ({ page }) => {
    await bootShim(page);
    await captureEvents(page);
    const row = await seedPending(page);

    const miss = await ipc(page, "agent_respond_ask", { askId: "ask-legacy", answer: "旧卡片答复" });
    expect(miss).toEqual({ ok: true, value: undefined });

    const hit = await ipc(page, "agent_respond_ask", { askId: row.askId, answer: "沪深300" });
    expect(hit.ok).toBe(true);

    const listed = await ipc(page, "list_bot_input_requests", { filter: {} });
    expect((listed as any).value).toHaveLength(1);
    expect((listed as any).value[0]).toMatchObject({ status: "answered", answer: "沪深300" });

    const evs = await events(page);
    expect(evs.map((e) => e.status)).toEqual(["pending", "answered"]);
  });
});

// ── B. 页面证据（B4 未落地：fixme 显式待办） ────────────────────────────

test.describe("P4-b 待答请求 · Bot 线程持久化卡片（待 B4 落地点亮）", () => {
  test.fixme("AC-B5-UI-1：Bot 线程提问 → 卡片（问题/选项）→ 作答 → 卡片转已答", async ({ page }) => {
    await page.addInitScript(installP3Shim, { bots: INPUT_BOTS, departments: [] });
    await page.goto("/chat?bot=bot-input-a");
    await page.evaluate(() => (window as any).__P3_TEST__.seedInputRequest({
      askId: "ask-ui-1", conversationId: "conv-bot-1", botId: "bot-input-a",
      question: "请确认标的池范围？", options: ["沪深300", "中证500"],
    }));

    const card = page.getByTestId("bot-input-request-card");
    await expect(card).toBeVisible({ timeout: 15_000 });
    await expect(page.getByTestId("bot-input-request-question")).toContainText("请确认标的池范围？");
    await expect(page.getByTestId("bot-input-request-options")).toContainText("沪深300");

    await page.getByTestId("bot-input-request-options").getByText("沪深300").click();
    await page.getByTestId("bot-input-request-submit").click();
    await expect(page.getByTestId("bot-input-request-status")).toContainText("已回答");
  });

  test.fixme("AC-B5-UI-2：刷新页面后 pending 卡从 list_bot_input_requests 恢复", async ({ page }) => {
    await page.addInitScript(installP3Shim, { bots: INPUT_BOTS, departments: [] });
    await page.goto("/chat?bot=bot-input-a");
    await page.evaluate(() => (window as any).__P3_TEST__.seedInputRequest({
      askId: "ask-ui-2", conversationId: "conv-bot-1", botId: "bot-input-a",
      question: "刷新后仍在？",
    }));

    await page.reload();
    await expect(page.getByTestId("bot-input-request-card")).toBeVisible({ timeout: 15_000 });
    await expect(page.getByTestId("bot-input-request-question")).toContainText("刷新后仍在？");
  });

  test.fixme("AC-B5-UI-3：超时 → 过期态（不可作答）", async ({ page }) => {
    await page.addInitScript(installP3Shim, { bots: INPUT_BOTS, departments: [] });
    await page.goto("/chat?bot=bot-input-a");
    const at = Date.now();
    await page.evaluate(([now]) => (window as any).__P3_TEST__.seedInputRequest({
      askId: "ask-ui-3", conversationId: "conv-bot-1", botId: "bot-input-a",
      question: "会过期的问题", createdAt: now - 400_000, expiresAt: now - 100_000,
    }), [at]);
    await page.evaluate(() => (window as any).__P3_TEST__.expireInputRequests());

    await expect(page.getByTestId("bot-input-request-status")).toContainText("已过期");
    await expect(page.getByTestId("bot-input-request-submit")).toBeDisabled();
  });

  test.fixme("AC-B5-UI-4：cancel → 取消态", async ({ page }) => {
    await page.addInitScript(installP3Shim, { bots: INPUT_BOTS, departments: [] });
    await page.goto("/chat?bot=bot-input-a");
    await page.evaluate(() => (window as any).__P3_TEST__.seedInputRequest({
      askId: "ask-ui-4", conversationId: "conv-bot-1", botId: "bot-input-a",
      question: "可取消的问题",
    }));

    await page.getByTestId("bot-input-request-cancel").click();
    await expect(page.getByTestId("bot-input-request-status")).toContainText("已取消");
  });

  test.fixme("AC-B5-UI-5：非 Bot 聊天 → 旧通用卡片与旧命令零变化", async ({ page }) => {
    await page.addInitScript(installP3Shim, { bots: INPUT_BOTS, departments: [] });
    await page.goto("/chat");

    // 通用聊天只走既有 agent-ask-user；不得出现持久化卡，也不得落 bot_input_requests 行
    await page.evaluate(() => (window as any).__P3_TEST__.emit("agent-ask-user", {
      askId: "ask-generic", question: "通用提问", options: null,
    }));
    await expect(page.getByTestId("bot-input-request-card")).toHaveCount(0);
    const rows = await page.evaluate(async () => {
      const t = (window as any).__P3_TEST__;
      return t.invoke("list_bot_input_requests", { filter: {} });
    });
    expect(rows).toHaveLength(0);
  });

  test.fixme("AC-B5-UI-6：/bots 侧栏待答数徽章随 pending/answered 事件增减", async ({ page }) => {
    await page.addInitScript(installP3Shim, { bots: INPUT_BOTS, departments: [] });
    await page.goto("/bots");
    await page.evaluate(() => (window as any).__P3_TEST__.seedInputRequest({
      askId: "ask-ui-6", conversationId: "conv-bot-1", botId: "bot-input-a",
      question: "侧栏计数？",
    }));

    await expect(page.getByTestId("bot-input-badge")).toContainText("1");
    const row = await page.evaluate(async () => {
      const t = (window as any).__P3_TEST__;
      const list = await t.invoke("list_bot_input_requests", { filter: { status: "pending" } });
      return t.invoke("answer_bot_input_request", { requestId: list[0].id, answer: "好" });
    });
    expect(row.status).toBe("answered");
    await expect(page.getByTestId("bot-input-badge")).toHaveCount(0);
  });
});