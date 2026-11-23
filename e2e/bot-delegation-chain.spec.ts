// SPDX-License-Identifier: AGPL-3.0-only

/**
 * P3 T7''' 多层委派链 e2e 骨架 —— 只验证页面可达与空态，不宣称覆盖链语义。
 *
 * 覆盖：
 *   1. /bots 可达，且无 Bot 时 P1 空态 `[data-testid="bot-empty-state"]` 仍在；
 *   2. 「编排」页签下委派链面板可达，且无链时 `bot-delegation-chain-empty` 可见。
 *
 * 明确不覆盖（等后端命令注册、前端面板接齐后另写；不用 skip/fixme 冒充覆盖）：
 *   depth 1→2→3 三层链、root 相同 / parent 链、L 前缀回执、同线程、
 *   C 再委派 MAX_DEPTH、B 委派回 A 的 CYCLE、同层第 3 单 BUSY、
 *   父 completed 子在途时 run 仍 running、整链取消、错误码继承。
 *
 * 选择器契约（src/components/bot/BotDelegationChainPanel.tsx + I18N-KEYS-P3.md）：
 *   · [data-testid="bot-empty-state"]（P1 沿用）
 *   · role=tab name="编排"（bot.workflow.title）
 *   · [data-testid="bot-delegation-chain-panel"] / bot-delegation-chain-empty
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
      id: "bot-p3-chain-skeleton",
      name: "委派链骨架 Bot",
      soul: "# P3 骨架",
    },
  ];
  return `(() => {
    localStorage.setItem("axagent_bots", ${JSON.stringify(JSON.stringify(bots))});
  })();`;
}

test.describe("Bot 多层委派链（P3 T7''' 骨架：可达 + 空态）", () => {
  test("骨架：/bots 可达且无 Bot 时 P1 空态保留", async ({ page }) => {
    await page.goto("/bots");
    await expect(page).toHaveURL(/\/bots(?:\?.*)?$/);
    await expect(page.getByTestId("bot-empty-state")).toBeVisible({ timeout: 15_000 });
  });

  test("骨架：编排页签下委派链面板可达且空链不丢", async ({ page }) => {
    await page.addInitScript(seedOneBotScript());
    await page.goto("/bots");
    await expect(page).toHaveURL(/\/bots(?:\?.*)?$/);

    const workflowTab = page.getByRole("tab", { name: "编排" });
    await expect(workflowTab).toBeVisible({ timeout: 15_000 });
    await workflowTab.click();

    await expect(page.getByTestId("bot-delegation-chain-panel")).toBeVisible({ timeout: 15_000 });
    await expect(page.getByTestId("bot-delegation-chain-empty")).toBeVisible({ timeout: 15_000 });
  });
});
