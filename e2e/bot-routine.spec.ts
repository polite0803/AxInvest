// SPDX-License-Identifier: AGPL-3.0-only

/**
 * P2-a T6'「Bot 例程」端到端验收（浏览器模式，browserMock）。
 *
 * ## 覆盖范围（PLAN-P2 §6 的浏览器可达子集）
 *
 * 纵向正路径：建例程（名称/提示词/cron/时区）→ 例程列表展示（cron·时区·下次运行）
 * → 会话产生 agent-done → 通知带 Bot 归属（localStorage + 铃铛「按 Bot 筛选」）
 * → BotsPage 列表未读角标 → 删 Bot 后例程级联消失（GAP G-10）。
 * 负路径：坏 cron（每分钟）被表单校验拒绝，模态不关闭。
 *
 * ## 为什么没有「到点自动跑 → 任务中心历史」
 *
 * 浏览器模式没有调度器与 provider 执行面；例程触发执行、审批档映射（ask 档
 * 写操作被拒、history.error 落地）由 Rust 侧锁定：`src-tauri/tests/bot_routine.rs`
 * （T2'，4 测）+ `bot_approval.rs` 纯函数表。本文件的断言对象是 mock 数据面
 * （`browserMockBots.ts` 例程命令）与 UI 装配的纵向一致性 —— 数据面契约另有
 * `src/lib/__tests__/browserMock.bots.test.ts` 锁定。
 *
 * ## 选择器契约
 *
 *   · `[data-testid="bot-list-item"]` / `bot-delete-btn`（P1 沿用）
 *   · `[data-testid="bot-routines-panel"]` / `bot-routine-add` / `bot-routine-item`
 *   · `[data-testid="bot-routine-modal"]`（表单）
 *   · Tab 标签「例程」（`bots.tabRoutines`）
 *   · 铃铛：`.ant-badge:has(.anticon-bell)`；筛选项「按 Bot 筛选 · <短id>」
 * 全部为已落地 UI 的稳定钩子；少任何一个以「元素不存在」明确失败，不静默跳过。
 */

import { expect, test } from "@playwright/test";

const BOT = { id: "bot-t6", name: "例程机器人", soul: "# 例程 SOUL" };

const BASE_BOT = {
  title: null,
  description: null,
  soulHash: "00000000",
  soulDrift: false,
  icon: "🤖",
  color: null,
  section: "default",
  modelId: null,
  providerId: null,
  effort: null,
  workdir: null,
  approvalMode: "full",
  agentProfileId: null,
  isEnabled: true,
  sortOrder: 0,
  createdAt: 1_700_000_000_000,
  updatedAt: 1_700_000_000_000,
};

/** 种入一个 Bot（localStorage 键形与 browserMockBots 一致：axagent_bots）。 */
function seedScript() {
  return `(() => {
    if (localStorage.getItem("__t6_routine_seeded")) { return; }
    localStorage.setItem("axagent_bots", JSON.stringify([
      { ...${JSON.stringify(BASE_BOT)}, id: ${JSON.stringify(BOT.id)}, name: ${JSON.stringify(BOT.name)}, soul: ${
    JSON.stringify(BOT.soul)
  } },
    ]));
    localStorage.setItem("__t6_routine_seeded", "1");
  })();`;
}

const botItems = (page: import("@playwright/test").Page) => page.locator('[data-testid="bot-list-item"]');

test.describe("Bot 例程（P2-a T6'）", () => {
  test("纵向：建例程 → 列表展示 → 通知带 Bot 归属 → 角标 → 删 Bot 级联", async ({ page }) => {
    await page.addInitScript(seedScript());

    // ── 1. 建例程（Asia/Shanghai，cron 30 8 * * 1-5）─────────────────────
    await page.goto("/bots");
    await botItems(page).filter({ hasText: BOT.name }).click();
    await page.getByRole("tab", { name: "例程" }).click();
    const panel = page.locator('[data-testid="bot-routines-panel"]');
    await expect(panel).toBeVisible({ timeout: 15_000 });

    await page.locator('[data-testid="bot-routine-add"]').click();
    // antd Modal 把 data-testid 放在无尺寸的 .ant-modal-root 包装层上（fixed 子元素
    // 不撑开父盒 ⇒ Playwright 判 0x0 为 hidden），故可见性断言定到 .ant-modal 本体。
    const modal = page.locator('[data-testid="bot-routine-modal"] .ant-modal');
    await expect(modal).toBeVisible();
    await modal.locator("input").first().fill("每日复盘");
    await modal.locator("textarea").fill("总结今日行情");
    await modal.locator("input").nth(1).fill("30 8 * * 1-5");
    // 时区下拉选 Asia/Shanghai（预设清单含此项）
    await modal.getByRole("combobox", { name: "时区" }).click();
    await page.locator(".ant-select-item").filter({ hasText: "Asia/Shanghai" }).first().click();
    await modal.locator(".ant-modal-footer .ant-btn-primary").click();

    // ── 2. 列表展示：名称 / cron / 时区 / 下次运行 ────────────────────────
    const item = page.locator('[data-testid="bot-routine-item"]').filter({ hasText: "每日复盘" });
    await expect(item).toHaveCount(1, { timeout: 15_000 });
    await expect(item).toContainText("30 8 * * 1-5");
    await expect(item).toContainText("Asia/Shanghai");
    await expect(item).toContainText("下次运行");

    // ── 3. 会话 agent-done → 通知带 botId（IPC-P2 §4 归属解析）────────────
    await page.goto(`/chat?bot=${BOT.id}`);
    await page.locator('[data-testid="new-conversation-btn"]').click();
    await expect(page.locator(".conv-item")).toHaveCount(1, { timeout: 15_000 });
    const input = page.locator('[data-testid="message-input"]');
    await expect(input).toBeVisible({ timeout: 15_000 });
    await input.fill("你好");
    await input.press("Enter");
    // 通知落库且带 Bot 归属（mock agent-done → agentStore.handleDone 解析归属）
    await page.waitForFunction(
      (botId) => {
        const raw = localStorage.getItem("axagent-notifications");
        const list = raw ? (JSON.parse(raw) as { botId?: string | null; read?: boolean }[]) : [];
        return list.some((n) => n.botId === botId && n.read === false);
      },
      BOT.id,
      { timeout: 30_000 },
    );

    // ── 4. BotsPage 未读角标 + 铃铛「按 Bot 筛选」────────────────────────
    await page.goto("/bots");
    const badge = botItems(page).filter({ hasText: BOT.name }).locator(".ant-badge-count");
    await expect(badge).toHaveText("1", { timeout: 15_000 });

    await page.locator(".ant-badge:has(.anticon-bell)").first().click();
    await expect(page.getByText(/按 Bot 筛选/)).toBeVisible({ timeout: 15_000 });

    // ── 5. 删 Bot：例程级联消失（GAP G-10），角标随列表项消失 ─────────────
    await botItems(page).filter({ hasText: BOT.name }).click();
    await page.locator('[data-testid="bot-delete-btn"]').click();
    await page.locator(".ant-popconfirm-buttons .ant-btn-primary").click();
    await expect(botItems(page)).toHaveCount(0, { timeout: 15_000 });

    const routines = await page.evaluate(() =>
      JSON.parse(localStorage.getItem("axagent_bot_routines") ?? "[]") as { botId?: string }[]
    );
    expect(routines).toHaveLength(0);
  });

  test("负路径：每分钟 cron 被表单校验拒绝，模态不关闭", async ({ page }) => {
    await page.addInitScript(seedScript());
    await page.goto("/bots");
    await botItems(page).filter({ hasText: BOT.name }).click();
    await page.getByRole("tab", { name: "例程" }).click();
    const panel = page.locator('[data-testid="bot-routines-panel"]');
    await expect(panel).toBeVisible({ timeout: 15_000 });

    await page.locator('[data-testid="bot-routine-add"]').click();
    // antd Modal 把 data-testid 放在无尺寸的 .ant-modal-root 包装层上（fixed 子元素
    // 不撑开父盒 ⇒ Playwright 判 0x0 为 hidden），故可见性断言定到 .ant-modal 本体。
    const modal = page.locator('[data-testid="bot-routine-modal"] .ant-modal');
    await expect(modal).toBeVisible();
    await modal.locator("input").first().fill("太频繁");
    await modal.locator("textarea").fill("每分钟跑一次");
    // 5 段但每分钟（分钟字段覆盖 0-59 全集）→ validateCronSyntax 拒绝
    await modal.locator("input").nth(1).fill("* * * * *");
    await modal.locator(".ant-modal-footer .ant-btn-primary").click();
    await expect(page.getByText(/Cron 表达式无效/).first()).toBeVisible({ timeout: 15_000 });
    await expect(modal).toBeVisible();

    // 未落库：例程清单仍为空
    const routines = await page.evaluate(() =>
      JSON.parse(localStorage.getItem("axagent_bot_routines") ?? "[]") as unknown[]
    );
    expect(routines).toHaveLength(0);
  });
});
