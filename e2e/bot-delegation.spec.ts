// SPDX-License-Identifier: AGPL-3.0-only

/**
 * P2-b T6″「Bot 委派 / 部门归属」端到端验收（浏览器模式，browserMock）。
 *
 * ## 覆盖范围（PLAN-P2B §6 的浏览器可达子集）
 *
 * 纵向正路径：建 Bot A(edits)/B(ask) → 建部门（Chief=A，成员 A+B）→ A 向 B 发起
 * dispatch → assignee(B) 面板见 queued → UI 取消（cancelled）→ 第二单在途 → 删 Bot B
 * → 在途单级联 cancelled、Chief 保留、B 从成员消失（ADR-P2B-6 / G-10 等价）。
 * 负路径：空部门名被表单校验拒绝，模态不关闭、不落库。
 *
 * 可见性口径（MODEL-P2B §2.3）：person 发起时 requesterBotId=NULL，面板按 botId
 * 过滤只命中 assignee，故在 B 的「委派」区验 UI；级联结果另直读 localStorage 数据面。
 *
 * ## 浏览器 mock 的边界（不做假绿断言）
 *
 * browserMockBots.ts 的 create_bot_assignment 只落 queued（无运行时/调度器），
 * 因此 queued→running→completed 状态机、ask 档写操作被拒（failed/ERR_PERMISSION）、
 * 300s 超时、并行门、回执正文落发起线程、consult 工具防环均由 Rust 侧锁定：
 * src-tauri/tests/bot_assignment.rs / src-tauri/tests/bot_routine.rs 与
 * axagent-runtime/crates/tools/src/tools/bot_delegate.rs 的单测。本文件只断
 * 数据面与 UI 装配的纵向一致性；数据面契约另有 src/lib/__tests__/browserMock.bots.test.ts。
 *
 * ## 选择器契约
 *
 *   · [data-testid="bot-list-item"] / bot-delete-btn（P1 沿用）
 *   · bot-department-manage / bot-department-modal / bot-department-name /
 *     bot-department-chief / bot-department-menu-<id> / bot-department-add-member
 *   · Tab「委派」/ bot-assignments-panel / bot-assignment-toggle-form /
 *     bot-assignment-assignee / bot-assignment-task / bot-assignment-send /
 *     bot-assignment-item[data-status] / bot-assignment-cancel
 * 少任何一个以「元素不存在」明确失败，不静默跳过。
 */

import { expect, type Page, test } from "@playwright/test";

const CHIEF = { id: "bot-t6b-chief", name: "统筹机器人甲", soul: "# 统筹 SOUL", approvalMode: "edits" };
const WORKER = { id: "bot-t6b-worker", name: "执行机器人乙", soul: "# 执行 SOUL", approvalMode: "ask" };
const DEPT = "研发一组";

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
  isEnabled: true,
  sortOrder: 0,
  createdAt: 1_700_000_000_000,
  updatedAt: 1_700_000_000_000,
};

/** 种入两个 Bot（localStorage 键形与 browserMockBots 一致：axagent_bots）。 */
function seedScript() {
  return `(() => {
    if (localStorage.getItem("__t6b_delegation_seeded")) { return; }
    const base = ${JSON.stringify(BASE_BOT)};
    localStorage.setItem("axagent_bots", JSON.stringify([
      { ...base, id: ${JSON.stringify(CHIEF.id)}, name: ${JSON.stringify(CHIEF.name)},
        soul: ${JSON.stringify(CHIEF.soul)}, approvalMode: ${JSON.stringify(CHIEF.approvalMode)} },
      { ...base, id: ${JSON.stringify(WORKER.id)}, name: ${JSON.stringify(WORKER.name)},
        soul: ${JSON.stringify(WORKER.soul)}, approvalMode: ${JSON.stringify(WORKER.approvalMode)} },
    ]));
    localStorage.setItem("__t6b_delegation_seeded", "1");
  })();`;
}

const botItems = (page: Page) => page.locator('[data-testid="bot-list-item"]');
const modal = (page: Page) => page.locator('[data-testid="bot-department-modal"] .ant-modal');
const option = (page: Page, name: string) =>
  page.locator(".ant-select-dropdown:visible .ant-select-item-option").filter({ hasText: name }).first();

async function chooseSelect(page: Page, testId: string, name: string) {
  await page.locator(`[data-testid="${testId}"]`).click();
  await option(page, name).click();
}

async function openAssignments(page: Page, botName: string) {
  await botItems(page).filter({ hasText: botName }).click();
  await page.getByRole("tab", { name: "委派" }).click();
  await expect(page.locator('[data-testid="bot-assignments-panel"]')).toBeVisible({ timeout: 15_000 });
}

async function sendAssignment(page: Page, taskText: string) {
  await page.locator('[data-testid="bot-assignment-toggle-form"]').click();
  await expect(page.locator('[data-testid="bot-assignment-form"]')).toBeVisible();
  await chooseSelect(page, "bot-assignment-assignee", WORKER.name);
  await page.locator('[data-testid="bot-assignment-task"]').fill(taskText);
  await expect(page.locator('[data-testid="bot-assignment-mode"]')).toContainText("派工");
  await page.locator('[data-testid="bot-assignment-send"]').click();
}

test.describe("Bot 部门归属与委派（P2-b T6″）", () => {
  test("纵向：建部门 → dispatch 委派 → 取消 → 删 Bot 级联 → Chief/成员保留", async ({ page }) => {
    await page.addInitScript(seedScript());
    await page.goto("/bots");
    await expect(botItems(page)).toHaveCount(2, { timeout: 15_000 });

    // ── 1. 建部门，Chief = A ────────────────────────────────────────────
    await page.locator('[data-testid="bot-department-manage"]').click();
    await expect(modal(page)).toBeVisible();
    await modal(page).locator('[data-testid="bot-department-name"]').fill(DEPT);
    await chooseSelect(page, "bot-department-chief", CHIEF.name);
    await modal(page).locator(".ant-modal-footer .ant-btn-primary").click();
    await expect(modal(page)).toBeHidden({ timeout: 15_000 });
    await expect(page.getByText(DEPT, { exact: true }).first()).toBeVisible({ timeout: 15_000 });

    // ── 2. 编辑部门：成员 A + B（一个 Bot 只属一个部门）──────────────────
    await page.locator('[data-testid^="bot-department-menu-"]').first().click();
    await page.locator(".ant-dropdown-menu-item").filter({ hasText: "编辑部门" }).click();
    await expect(modal(page)).toBeVisible();
    for (const bot of [CHIEF, WORKER]) {
      await chooseSelect(page, "bot-department-add-member", bot.name);
      await expect(modal(page).locator(`[data-testid="bot-department-member-${bot.id}"]`))
        .toHaveCount(1, { timeout: 15_000 });
    }
    await modal(page).locator(".ant-modal-footer .ant-btn-primary").click();
    await expect(modal(page)).toBeHidden({ timeout: 15_000 });

    // ── 3. A 视角发起 dispatch → B；assignee 面板见 queued ──────────────
    await openAssignments(page, CHIEF.name);
    await sendAssignment(page, "任务甲：汇总今日行情");
    await openAssignments(page, WORKER.name);
    const first = page.locator('[data-testid="bot-assignment-item"]').filter({ hasText: "任务甲" });
    await expect(first).toHaveCount(1, { timeout: 15_000 });
    await expect(first).toHaveAttribute("data-status", "queued");
    await expect(first).toContainText("任务甲");
    // UI 无执行器：mock 只落 queued，不伪造 running/completed 与回执线程。

    // ── 4. UI 取消在途单 → cancelled，终态禁用取消 ──────────────────────
    await first.locator('[data-testid="bot-assignment-cancel"]').click();
    await page.locator(".ant-popconfirm-buttons .ant-btn-primary").click();
    await expect(first).toHaveAttribute("data-status", "cancelled", { timeout: 15_000 });
    await expect(first.locator('[data-testid="bot-assignment-cancel"]')).toHaveCount(0);

    // ── 5. 第二单留在途，验证 delete_bot 级联（ADR-P2B-6）────────────────
    await openAssignments(page, CHIEF.name);
    await sendAssignment(page, "任务乙：生成风险清单");
    await openAssignments(page, WORKER.name);
    const inFlight = page.locator('[data-testid="bot-assignment-item"]').filter({ hasText: "任务乙" });
    await expect(inFlight).toHaveAttribute("data-status", "queued", { timeout: 15_000 });
    await expect(page.locator('[data-testid="bot-assignment-item"]')).toHaveCount(2);

    await botItems(page).filter({ hasText: WORKER.name }).click();
    await page.locator('[data-testid="bot-delete-btn"]').click();
    await page.locator(".ant-popconfirm-buttons .ant-btn-primary").click();
    await expect(botItems(page)).toHaveCount(1, { timeout: 15_000 });
    await expect(botItems(page).filter({ hasText: WORKER.name })).toHaveCount(0);

    // B 已删，assignee 面板随 Bot 消失；级联结果按数据面口径直读 localStorage。
    await expect(page.getByText(DEPT, { exact: true }).first()).toBeVisible({ timeout: 15_000 });
    await expect(botItems(page).filter({ hasText: CHIEF.name })).toHaveCount(1);

    // 部门保留、Chief 保留（A 未被删）、B 从成员消失；两条在途单均转 cancelled。
    const state = await page.evaluate(() => ({
      departments: JSON.parse(localStorage.getItem("axagent_bot_departments") ?? "[]") as {
        name: string;
        chiefBotId: string | null;
        botIds: string[];
      }[],
      assignments: JSON.parse(localStorage.getItem("axagent_bot_assignments") ?? "[]") as {
        status: string;
        assigneeBotId: string;
      }[],
    }));
    const dept = state.departments.find((d) => d.name === DEPT);
    expect(dept).toBeDefined();
    expect(dept?.chiefBotId).toBe(CHIEF.id);
    expect(dept?.botIds).toContain(CHIEF.id);
    expect(dept?.botIds).not.toContain(WORKER.id);
    expect(state.assignments.filter((a) => a.assigneeBotId === WORKER.id && a.status === "cancelled"))
      .toHaveLength(2);
  });

  test("负路径：空部门名被表单校验拒绝，模态不关闭且不落库", async ({ page }) => {
    await page.addInitScript(seedScript());
    await page.goto("/bots");
    await expect(botItems(page)).toHaveCount(2, { timeout: 15_000 });

    await page.locator('[data-testid="bot-department-manage"]').click();
    await expect(modal(page)).toBeVisible();
    await modal(page).locator('[data-testid="bot-department-name"]').fill("   ");
    await modal(page).locator(".ant-modal-footer .ant-btn-primary").click();
    await expect(page.getByText("请输入部门名称").first()).toBeVisible({ timeout: 15_000 });
    await expect(modal(page)).toBeVisible();

    const departments = await page.evaluate(() =>
      JSON.parse(localStorage.getItem("axagent_bot_departments") ?? "[]") as unknown[]
    );
    expect(departments).toHaveLength(0);
  });
});
