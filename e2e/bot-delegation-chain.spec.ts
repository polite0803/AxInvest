// SPDX-License-Identifier: AGPL-3.0-only

/**
 * P3 T7''' 多层委派链 e2e：骨架（可达 + 空态）+ 链语义与门拒绝。
 *
 * 覆盖：
 *   1. /bots 可达，且无 Bot 时 P1 空态 `[data-testid="bot-empty-state"]` 仍在；
 *   2. 「编排」页签下委派链面板可达，且无链时 `bot-delegation-chain-empty` 可见；
 *   3. depth 1→2→3、root/parent、L 前缀回执、cycle/maxDepth/busy、
 *      根 completed 子孙在途时 run 不终态、整链取消。
 *
 * 边界：浏览器面用测试 IPC 替身驱动真实页面；运行时错误码继承由 Rust 链测试锁定。
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
    await page.goto("/bots/workflows");
    await expect(page).toHaveURL(/\/bots\/workflows(?:\?.*)?$/);
    await page.locator('[data-testid="bot-workflow-tabs"] .ant-tabs-tab').nth(3).click();

    await expect(page.getByTestId("bot-delegation-chain-panel")).toBeVisible({ timeout: 15_000 });
    await expect(page.getByTestId("bot-delegation-chain-empty")).toBeVisible({ timeout: 15_000 });
  });
});

// ── P3 收口：多层委派链（depth 1→2→3 / chain 字段 / L 回执 / 门拒绝） ────
import { installP3Shim } from "./helpers/p3-tauri-shim";

const P3_CHAIN_BOTS = [
  { ...BASE_BOT, id: "bot-chain-a", name: "甲" },
  { ...BASE_BOT, id: "bot-chain-b", name: "乙" },
  { ...BASE_BOT, id: "bot-chain-c", name: "丙" },
  { ...BASE_BOT, id: "bot-chain-d", name: "丁" },
  { ...BASE_BOT, id: "bot-chain-e", name: "戊" },
];

test("P3 多层链：depth 1→2→3、root/parent 字段、L 前缀回执、cycle/maxDepth/busy 拒绝", async ({ page }) => {
  await page.addInitScript(installP3Shim, { bots: P3_CHAIN_BOTS });
  await page.goto("/bots/workflows");
  await page.getByTestId("onboarding-skip").click({ timeout: 8_000 }).catch(() => undefined);
  await expect(page.getByTestId("bot-workflows-page")).toBeVisible({ timeout: 15_000 });

  // 种三层链：A→B(depth1)、B→C(depth2)、C→D(depth3)，同一 root。
  const chain = await page.evaluate(async () => {
    const t = (window as any).__P3_TEST__;
    const root = await t.invoke("create_bot_assignment", {
      assigneeBotId: "bot-chain-b", taskText: "一层派工", stepKey: "root-step", origin: "person",
    });
    const mid = await t.invoke("create_bot_assignment", {
      parentAssignmentId: root.id, assigneeBotId: "bot-chain-c",
      taskText: "二层派工", stepKey: "mid-step", origin: "bot", requesterBotId: "bot-chain-b",
    });
    const leaf = await t.invoke("create_bot_assignment", {
      parentAssignmentId: mid.id, assigneeBotId: "bot-chain-d",
      taskText: "三层派工", stepKey: "leaf-step", origin: "bot", requesterBotId: "bot-chain-c",
    });
    const rows = await t.invoke("list_bot_assignments", { rootId: root.rootId });
    return { root, mid, leaf, rows: rows.map((a: any) => ({
      id: a.id, rootId: a.rootId, parentId: a.parentId, depth: a.depth,
      origin: a.origin, resultText: a.resultText, status: a.status,
    })) };
  });
  expect([chain.root.depth, chain.mid.depth, chain.leaf.depth]).toEqual([1, 2, 3]);
  expect(chain.rows).toHaveLength(3);
  expect(new Set(chain.rows.map((r: any) => r.rootId)).size).toBe(1);
  expect(chain.mid.parentId).toBe(chain.root.id);
  expect(chain.leaf.parentId).toBe(chain.mid.id);
  expect(chain.rows.map((r: any) => r.origin)).toEqual(["person", "bot", "bot"]);
  expect(chain.mid.resultText).toContain("[L2 · mid-step · 丙]");
  expect(chain.leaf.resultText).toContain("[L3 · leaf-step · 丁]");

  // UI：三层分组、badge、链顺序、L 前缀回执都可见。
  await page.locator('[data-testid="bot-workflow-tabs"] .ant-tabs-tab').nth(3).click();
  const panel = page.getByTestId("bot-delegation-chain-panel");
  await expect(panel).toBeVisible({ timeout: 15_000 });
  for (const depth of [1, 2, 3]) {
    await expect(page.locator(`[data-testid="bot-delegation-depth-group"][data-depth="${depth}"]`))
      .toHaveCount(1);
  }
  await expect(page.getByTestId("bot-assignment-item")).toHaveCount(3);
  await expect(page.getByTestId("bot-delegation-chain-order"))
    .toContainText(`${chain.root.id} > ${chain.mid.id} > ${chain.leaf.id}`);
  await expect(panel).toContainText("[L2 · mid-step · 丙]");
  await expect(panel).toContainText("[L3 · leaf-step · 丁]");

  // 门：cycle（祖先命中）/ maxDepth（depth=3 再往下）/ 同层第 3 单 BUSY。
  const gates = await page.evaluate(async () => {
    const t = (window as any).__P3_TEST__;
    const out: Record<string, string> = {};
    const attempt = async (key: string, args: any) => {
      try { await t.invoke("create_bot_assignment", args); out[key] = "NO_ERROR"; }
      catch (e: any) { out[key] = String(e.message ?? e); }
    };
    await attempt("cycle", {
      parentAssignmentId: t.state.assignments[1].id, assigneeBotId: "bot-chain-b", taskText: "回派祖先",
    });
    await attempt("maxDepth", {
      parentAssignmentId: t.state.assignments[2].id, assigneeBotId: "bot-chain-e", taskText: "第四层",
    });
    await attempt("busy1", {
      parentAssignmentId: t.state.assignments[0].id, assigneeBotId: "bot-chain-d",
      taskText: "同层第一单", status: "queued",
    });
    await attempt("busy2", {
      parentAssignmentId: t.state.assignments[0].id, assigneeBotId: "bot-chain-e",
      taskText: "同层第二单", status: "queued",
    });
    await attempt("busy3", {
      parentAssignmentId: t.state.assignments[0].id, assigneeBotId: "bot-chain-c",
      taskText: "同层第三单", status: "queued",
    });
    return out;
  });
  expect(gates.cycle).toContain("BOT_DELEGATE_CYCLE");
  expect(gates.maxDepth).toContain("BOT_DELEGATE_MAX_DEPTH");
  expect(gates.busy1).not.toContain("BOT_DELEGATE");
  expect(gates.busy2).not.toContain("BOT_DELEGATE");
  expect(gates.busy3).toContain("BOT_DELEGATE_BUSY");

  // 子树等待：把根 assignment 设为 completed、L2/L3 保持 queued，
  // 前端 effectiveRunStatus 必须把 run 投影为执行中；子孙全终态后才显示完成。
  const waitRunId = await page.evaluate(async () => {
    const t = (window as any).__P3_TEST__;
    const wf = await t.invoke("create_bot_workflow", {
      name: "链等待验证", ownerScope: "instance", steps: [], maxConcurrency: 2, totalTimeoutSecs: 60,
    });
    const run = await t.invoke("run_bot_workflow", {
      workflowId: wf.id, requesterConversationId: "thread-chain-wait",
    });
    const chainIds = [t.state.assignments[0].id, t.state.assignments[1].id, t.state.assignments[2].id];
    for (const a of t.state.assignments) {
      if (chainIds.includes(a.id)) a.workflowRunId = run.id;
      if (a.id === chainIds[0]) a.status = "completed";
      if (chainIds.slice(1).includes(a.id)) a.status = "queued";
    }
    run.status = "completed"; // 根 completed 不应关 run：前端按在途子孙投影为 running
    t.emit("bot-workflow-run-updated", run);
    t.emit("bot-assignment-updated", {});
    return run.id;
  });
  await page.locator('[data-testid="bot-workflow-tabs"] .ant-tabs-tab').nth(2).click();
  const waitingRun = page.getByTestId("bot-workflow-run-item").filter({ hasText: waitRunId });
  await expect(waitingRun).toHaveAttribute("data-status", "running", { timeout: 15_000 });
  await expect(waitingRun.getByTestId("bot-workflow-run-status")).toContainText("执行中");
  await page.evaluate((runId) => {
    const t = (window as any).__P3_TEST__;
    for (const a of t.state.assignments) {
      if (a.workflowRunId === runId && a.status !== "completed") a.status = "completed";
    }
    t.emit("bot-assignment-updated", {});
  }, waitRunId);
  await expect(waitingRun).toHaveAttribute("data-status", "completed", { timeout: 15_000 });

  // 整链取消：根步骤在途时 run 未完成；取消后同 root 子孙一并终态。
  const cancelResult = await page.evaluate(async () => {
    const t = (window as any).__P3_TEST__;
    const rootId = t.state.assignments[0].rootId;
    for (const a of t.state.assignments) {
      if (a.rootId === rootId) a.status = "queued";
    }
    await t.invoke("cancel_bot_assignment", { assignmentId: t.state.assignments[0].id });
    const ids = [t.state.assignments[0].id, t.state.assignments[1].id, t.state.assignments[2].id];
    const rows = await t.invoke("list_bot_assignments", { rootId });
    return rows.filter((a: any) => ids.includes(a.id)).map((a: any) => a.status);
  });
  expect(cancelResult).toEqual(["cancelled", "cancelled", "cancelled"]);
});
