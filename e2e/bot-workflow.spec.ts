// SPDX-License-Identifier: AGPL-3.0-only

/**
 * P3 T7''' 编排 e2e 骨架 —— 只验证页面可达与空态，不宣称覆盖纵向场景。
 *
 * 覆盖：
 *   1. /bots 可达，且无 Bot 时 P1 空态 `[data-testid="bot-empty-state"]` 仍在；
 *   2. 「编排」页签可达，且无编排时 `bot.workflow.empty` 文案可见。
 *
 * 明确不覆盖（等后端命令注册、前端面板接齐后另写；不用 skip/fixme 冒充覆盖）：
 *   三步骤链、变量传递、queued→running→completed、单线程回执、失败跳过、
 *   取消、cron 定时、MEMBER_CHANGED/ERR_PERMISSION、并发门、总超时。
 *
 * 选择器契约（docs/bot-integration/P3/IPC-P3.md + I18N-KEYS-P3.md）：
 *   · [data-testid="bot-empty-state"]（P1 沿用）
 *   · role=tab name="编排"（bot.workflow.title）
 *   · 文案「暂无编排」（bot.workflow.empty，zh-CN）
 * 缺任一稳定钩子时以「元素不存在」明确失败，不静默跳过。
 */

import { expect, test } from "@playwright/test";

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
  agentProfileId: null,
  approvalMode: "full",
  isEnabled: true,
  sortOrder: 0,
  createdAt: 1_700_000_000_000,
  updatedAt: 1_700_000_000_000,
};

function seedOneBotScript() {
  const bots = [
    {
      ...BASE_BOT,
      id: "bot-p3-workflow-skeleton",
      name: "编排骨架 Bot",
      soul: "# P3 骨架",
    },
  ];
  return `(() => {
    localStorage.setItem("axagent_bots", ${JSON.stringify(JSON.stringify(bots))});
  })();`;
}

test.describe("Bot 编排（P3 T7''' 骨架：可达 + 空态）", () => {
  test("骨架：/bots 可达且无 Bot 时 P1 空态保留", async ({ page }) => {
    await page.goto("/bots");
    await expect(page).toHaveURL(/\/bots(?:\?.*)?$/);
    await expect(page.getByTestId("bot-empty-state")).toBeVisible({ timeout: 15_000 });
  });

  test("骨架：编排页签可达且无编排空态保留", async ({ page }) => {
    await page.addInitScript(seedOneBotScript());
    await page.goto("/bots");
    await expect(page).toHaveURL(/\/bots(?:\?.*)?$/);

    const workflowTab = page.getByRole("tab", { name: "编排" });
    await expect(workflowTab).toBeVisible({ timeout: 15_000 });
    await workflowTab.click();
    await expect(page.getByText("暂无编排", { exact: true })).toBeVisible({ timeout: 15_000 });
  });
});
