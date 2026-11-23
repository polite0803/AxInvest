// SPDX-License-Identifier: AGPL-3.0-only

/**
 * P3 编排 e2e：原 T7''' 骨架（可达 + 空态）+ 收口纵向场景
 * （create→run→steps/依赖/变量→单线程回执→失败跳过→取消→cron）。
 *
 * 覆盖：
 *   1. /bots 可达，且无 Bot 时 P1 空态 `[data-testid="bot-empty-state"]` 仍在；
 *   2. 「编排」页签可达，且无编排时 `bot.workflow.empty` 文案可见；
 *   3. 两步链 + dependsOn + {{step.output}} + 步骤/L2 回执 + 取消 + cron 登记 + failed/skipped UI。
 *
 * 边界：浏览器面用测试 IPC 替身驱动真实页面，运行时状态机由 Rust bot_workflow 测试锁定；
 *   未覆盖 MEMBER_CHANGED/ERR_PERMISSION、并发门、总超时（同上）。
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
    // P3 编排已从 /bots 的页签迁到独立路由 /bots/workflows（前端 767557bb1）。
    await page.goto("/bots/workflows");
    await expect(page).toHaveURL(/\/bots\/workflows(?:\?.*)?$/);
    await expect(page.getByTestId("bot-workflow-tabs")).toBeVisible({ timeout: 15_000 });
    await expect(page.getByText("暂无编排", { exact: true })).toBeVisible({ timeout: 15_000 });
  });
});

// ── P3 收口：纵向流程（create → run → steps → 回执 → 取消 → cron） ──────
// 用测试 IPC 替身驱动真实编排页；生产 src/ 未改。Rust 运行时状态机另由 cargo 测试锁定。
import { installP3Shim } from "./helpers/p3-tauri-shim";

const P3_BOTS = [
  { ...BASE_BOT, id: "bot-p3-a", name: "采集助手" },
  { ...BASE_BOT, id: "bot-p3-b", name: "分析助手" },
  { ...BASE_BOT, id: "bot-p3-c", name: "复核助手" },
];

async function pickOption(page: any, testId: string, label: string) {
  await page.getByTestId(testId).click();
  await page.locator('.ant-select-item-option[title="' + label + '"]').last().dispatchEvent("click");
}

test("P3 纵向：建编排 → 运行 → 步骤输出/回执 → 取消 → cron 触发登记", async ({ page }) => {
  await page.addInitScript(installP3Shim, { bots: P3_BOTS });
  await page.goto("/bots/workflows");
  await page.getByTestId("onboarding-skip").click({ timeout: 8_000 }).catch(() => undefined);
  await expect(page.getByTestId("bot-workflows-page")).toBeVisible({ timeout: 15_000 });

  // 1) create：两步链 + 依赖 + 变量引用
  await page.getByTestId("bot-workflow-new").click();
  await page.getByTestId("bot-workflow-name").fill("投研两步编排");
  await page.getByTestId("bot-workflow-description").fill("collect → analyze");
  await page.getByTestId("bot-workflow-add-step").click();
  const cards = page.getByTestId("bot-workflow-step-card");
  await expect(cards).toHaveCount(2);

  await cards.nth(0).getByTestId("bot-workflow-step-key").fill("collect");
  await cards.nth(0).locator('[data-testid="bot-workflow-step-bot"]').click();
  await page.locator('.ant-select-item-option[title="采集助手"]').last().dispatchEvent("click");
  await cards.nth(0).getByTestId("bot-workflow-step-prompt").fill("采集 {{symbol}} 行情");

  await cards.nth(1).getByTestId("bot-workflow-step-key").fill("analyze");
  await cards.nth(1).locator('[data-testid="bot-workflow-step-bot"]').click();
  await page.locator('.ant-select-item-option[title="分析助手"]').last().dispatchEvent("click");
  await cards.nth(1).getByTestId("bot-workflow-step-prompt").fill("分析 {{collect.output}}");
  await cards.nth(1).getByTestId("bot-workflow-step-depends").getByText("collect").click();
  await page.getByTestId("bot-workflow-save").click();

  await expect(page.getByTestId("bot-workflow-item")).toHaveCount(1, { timeout: 15_000 });
  const created = await page.evaluate(() => {
    const log = (window as any).__P3_TEST__.state.log;
    return log.find((e: any) => e.cmd === "create_bot_workflow")?.args?.input;
  });
  expect(created).toMatchObject({ name: "投研两步编排", ownerScope: "instance" });
  expect(created.steps).toHaveLength(2);
  expect(created.steps[1].dependsOn).toContain("collect");
  expect(created.steps[1].promptTemplate).toContain("{{collect.output}}");

  // 2) run：手工触发带显式线程 id 键；queued → running + 步骤输出写回 context
  await page.locator('[data-testid="bot-workflow-tabs"] .ant-tabs-tab').nth(2).click();
  await page.getByTestId("bot-workflow-run").click();
  await expect(page.getByTestId("bot-workflow-run-item")).toHaveCount(1, { timeout: 15_000 });
  await expect(page.getByTestId("bot-workflow-run-status")).toContainText("执行中");
  await page.getByTestId("bot-workflow-run-outputs-toggle").click();
  const outputs = page.getByTestId("bot-workflow-run-outputs");
  await expect(outputs).toContainText("[步骤 collect · 采集助手]");
  await expect(outputs).toContainText("[步骤 analyze · 分析助手]");
  await expect(outputs).toContainText("{{collect.output}}");

  const runEvidence = await page.evaluate(() => {
    const t = (window as any).__P3_TEST__;
    return {
      runArgs: t.state.log.find((e: any) => e.cmd === "run_bot_workflow")?.args,
      assignments: t.state.assignments.map((a: any) => ({
        rootId: a.rootId, parentId: a.parentId, depth: a.depth,
        stepKey: a.stepKey, resultText: a.resultText,
      })),
      receipts: t.state.receipts.map((r: any) => r.text),
      eventSubscriptions: t.listeners(),
    };
  });
  // 手工触发显式给线程键（无当前线程时为 null），不做隐式建线程。
  expect(runEvidence.runArgs).toHaveProperty("workflowId");
  expect(runEvidence.runArgs).toHaveProperty("requesterConversationId");
  expect(Object.keys(runEvidence.runArgs).sort()).toEqual(["requesterConversationId", "workflowId"]);
  expect(runEvidence.assignments.filter((a: any) => a.depth === 1)).toHaveLength(2);
  expect(runEvidence.assignments.filter((a: any) => a.depth === 2)).toHaveLength(1);
  expect(new Set(runEvidence.assignments.map((a: any) => a.rootId)).size).toBe(1);
  expect(runEvidence.receipts.some((text: string) => text.startsWith("[步骤 collect ·"))).toBe(true);
  expect(runEvidence.receipts.some((text: string) => text.startsWith("[L2 ·"))).toBe(true);
  expect(Object.keys(runEvidence.eventSubscriptions)).toEqual(
    expect.arrayContaining(["bot-workflow-run-updated", "bot-assignment-updated"]));

  // 3) cancel：终态再取消必须有语义错误码（不静默成功）
  await page.getByTestId("bot-workflow-run-cancel").click();
  await expect(page.getByTestId("bot-workflow-run-status")).toContainText("已取消");
  const cancelError = await page.evaluate(async () => {
    const t = (window as any).__P3_TEST__;
    const id = t.state.runs[0].id;
    try { await t.invoke("cancel_bot_workflow_run", { runId: id }); return "NO_ERROR"; }
    catch (e: any) { return String(e.message ?? e); }
  });
  expect(cancelError).toContain("BOT_WORKFLOW_CANCEL_INVALID");

  // 4) cron：task_type / params.workflowId / botId=null 登记
  await pickOption(page, "bot-workflow-schedule-workflow", "投研两步编排");
  await page.getByTestId("bot-workflow-schedule-cron").fill("0 9 * * 1-5");
  await page.getByTestId("bot-workflow-schedule-timezone").fill("Asia/Shanghai");
  await page.getByTestId("bot-workflow-schedule-create").click();
  const schedule = await page.evaluate(() => {
    const t = (window as any).__P3_TEST__;
    return { task: t.state.schedules[0], workflowId: t.state.workflows[0].id };
  });
  expect(schedule.task).toMatchObject({
    taskType: "bot_workflow", cronExpression: "0 9 * * 1-5", timezone: "Asia/Shanghai",
    botId: null, params: { workflowId: schedule.workflowId },
  });

  // 5) 契约负路径：手工触发缺线程 id 键 → ERR_VALIDATION（无隐式建线程）
  const validationError = await page.evaluate(async () => {
    const t = (window as any).__P3_TEST__;
    try {
      await t.invoke("run_bot_workflow", { workflowId: t.state.workflows[0].id });
      return "NO_ERROR";
    } catch (e: any) { return String(e.message ?? e); }
  });
  expect(validationError).toContain("ERR_VALIDATION");

  // 6) 失败跳过：同一编排再起一 run，后端 DTO 带 failed + skipped 时 UI 显示失败态与跳过清单。
  //    运行时 skip 判据由 Rust test_5_continue_skips_downstream_but_finishes_independent_branch 锁定。
  const failureRunId = await page.evaluate(async (workflowId) => {
    const t = (window as any).__P3_TEST__;
    const run = await t.invoke("run_bot_workflow", {
      workflowId, requesterConversationId: "thread-failure-skip",
    });
    run.status = "failed";
    run.errorCode = "BOT_WORKFLOW_STEP_BOT_MISSING";
    run.context.skipped = ["analyze"];
    for (const a of t.state.assignments) {
      if (a.workflowRunId === run.id && a.stepKey === "analyze") a.status = "failed";
    }
    t.emit("bot-workflow-run-updated", run);
    t.emit("bot-assignment-updated", {});
    return run.id;
  }, schedule.workflowId);
  const failedRun = page.getByTestId("bot-workflow-run-item").filter({ hasText: failureRunId });
  await expect(failedRun).toHaveAttribute("data-status", "failed", { timeout: 15_000 });
  await failedRun.getByTestId("bot-workflow-run-outputs-toggle").click();
  await expect(failedRun.getByTestId("bot-workflow-run-skipped")).toContainText("analyze");
});
