// SPDX-License-Identifier: AGPL-3.0-only

/**
 * P4-a 部门运行配额 e2e（PLAN-P4 §6.3 A5 场景）。
 *
 * 本文件分两类，边界写死在这里，避免把替身自检当成产品证据：
 *
 * A. 契约自检（现在可跑，`test(...)`）：
 *    只验证 `e2e/helpers/p3-tauri-shim.ts` 按 IPC-P4 §1/§4 + ADR-P4 D2/D3 实现的
 *    6 条命令与错误码语义（默认值 / 范围校验 / 拒绝不裁剪 / 两部门隔离 / 运行层上限）。
 *    它证明「测试替身与冻结契约一致」，不证明 Rust 命令层行为；
 *    Rust 侧行为由 `src-tauri/tests/bot_quota.rs` 锁定。
 *
 * B. 页面证据（A4 已落盘，2026-11-25 复核 HEAD 749203835）：
 *    BotsPage 部门弹窗里的 BotQuotaPanel（默认值提示 / 越界预校验 / 保存回显 / 恢复默认）。
 *    ✔ 「默认值 + isDefault 提示」与「保存回显 + 恢复默认」两条已点亮为 `test`
 *      （testid 全齐：bot-quota-panel、六项输入、save/reset/default-hint/error、
 *       bot-department-quota-open）。
 *    ✘ 「越界前端预校验 + 后端码映射」保留 `test.fixme`，原因见该用例上方注释：
 *      实现只渲染本地化字段标签，不含断言要求的原始字段名；且前端预校验在提交前拦截，
 *      UI 无路径把后端 BOT_QUOTA_INVALID 暴露到 bot-quota-error。待契约/产品裁决后再点亮，
 *      不迁就改断言。
 *
 * 未在本文件覆盖：cron 触发超配额 → history 失败（调度器闭包在 init/services.rs，
 * 浏览器面无入口）；该断言落在 `src-tauri/tests/bot_quota.rs` 的 cron_gate 用例。
 */

import { expect, test, type Page } from "@playwright/test";
import { installP3Shim } from "./helpers/p3-tauri-shim";

const BASE_BOT = {
  title: null, description: null, soulHash: "00000000", soulDrift: false, icon: "🤖",
  color: null, section: "default", modelId: null, providerId: null, effort: null,
  workdir: null, agentProfileId: null, approvalMode: "full", isEnabled: true, sortOrder: 0,
  createdAt: 1_700_000_000_000, updatedAt: 1_700_000_000_000,
};

const QUOTA_BOTS = [
  { ...BASE_BOT, id: "bot-quota-a", name: "配额助手 A" },
  { ...BASE_BOT, id: "bot-quota-b", name: "配额助手 B" },
];
const QUOTA_DEPARTMENTS = [
  { id: "dept-alpha", name: "投研一部" },
  { id: "dept-beta", name: "投研二部" },
];

/** 只等替身注入完成，不依赖页面结构：契约自检不应因 UI 改版而假红。 */
async function bootShim(page: Page) {
  await page.addInitScript(installP3Shim, {
    bots: QUOTA_BOTS, departments: QUOTA_DEPARTMENTS,
  });
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

const step = (stepKey: string, botId = "bot-quota-a") => ({
  stepKey, botId, promptTemplate: "处理 " + stepKey,
});

const deptWorkflow = (over: Record<string, any> = {}) => ({
  name: "部门编排", ownerScope: "department", departmentId: "dept-alpha",
  maxConcurrency: 1, totalTimeoutSecs: 60,
  steps: [step("s1")], ...over,
});

const countCalls = (page: Page, cmd: string) => page.evaluate(
  (c) => (window as any).__P3_TEST__.state.log.filter((e: any) => e.cmd === c).length, cmd);

/** 首启引导层会拦截指针事件（与 bot-workflow/bot-delegation-chain 同口径）；无引导时静默跳过。 */
async function dismissOnboarding(page: Page) {
  await page.getByTestId("onboarding-skip").click({ timeout: 8_000 }).catch(() => undefined);
}

// ── A. 契约自检（现在可跑） ─────────────────────────────────────────────

test.describe("P4-a 配额 · IPC 契约自检（shim）", () => {
  test("命令 1/3：无行 get 返回 P3 默认值 + isDefault=true；reset 幂等", async ({ page }) => {
    await bootShim(page);

    const got = await ipc(page, "get_bot_department_quota", { departmentId: "dept-alpha" });
    expect(got.ok).toBe(true);
    expect((got as any).value).toEqual({
      departmentId: "dept-alpha",
      maxConcurrency: 2, totalTimeoutSecs: 900, maxSteps: 12,
      maxInFlightPerDepth: 2, stepTimeoutSecs: 300, maxConcurrentRuns: 2,
      isDefault: true, updatedAt: null,
    });

    // IPC-P4 §1：部门不存在 → BOT_DEPARTMENT_NOT_FOUND
    const missing = await ipc(page, "get_bot_department_quota", { departmentId: "dept-ghost" });
    expect(missing).toEqual({ ok: false, error: "BOT_DEPARTMENT_NOT_FOUND" });

    // 无行时 reset 也必须 Ok（幂等），且返回类型为 () 不是 DTO
    const reset = await ipc(page, "reset_bot_department_quota", { departmentId: "dept-alpha" });
    expect(reset).toEqual({ ok: true, value: undefined });
  });

  test("命令 2：逐字段越界 → BOT_QUOTA_INVALID:字段；合法保存回读 isDefault=false", async ({ page }) => {
    await bootShim(page);

    const outOfRange: Array<[string, number]> = [
      ["maxConcurrency", 5], ["totalTimeoutSecs", 59], ["maxSteps", 13],
      ["maxInFlightPerDepth", 0], ["stepTimeoutSecs", 901], ["maxConcurrentRuns", 9],
    ];
    for (const [field, value] of outOfRange) {
      const res = await ipc(page, "set_bot_department_quota", {
        input: { departmentId: "dept-alpha", [field]: value },
      });
      expect(res, field).toEqual({ ok: false, error: "BOT_QUOTA_INVALID:" + field });
    }

    const saved = await ipc(page, "set_bot_department_quota", {
      input: {
        departmentId: "dept-alpha", maxConcurrency: 1, totalTimeoutSecs: 60, maxSteps: 3,
        maxInFlightPerDepth: 1, stepTimeoutSecs: 120, maxConcurrentRuns: 1,
      },
    });
    expect(saved.ok).toBe(true);
    expect((saved as any).value).toMatchObject({
      departmentId: "dept-alpha", maxConcurrency: 1, totalTimeoutSecs: 60, maxSteps: 3,
      maxInFlightPerDepth: 1, stepTimeoutSecs: 120, maxConcurrentRuns: 1, isDefault: false,
    });

    // 回读 == 写入（保存回显的数据面）
    const readBack = await ipc(page, "get_bot_department_quota", { departmentId: "dept-alpha" });
    expect((readBack as any).value).toMatchObject({ maxSteps: 3, maxConcurrency: 1, isDefault: false });
    expect((readBack as any).value.updatedAt).toBeGreaterThan(0);
  });

  test("AC-A5-1：设 1 并发/60s/3 步 → 4 步编排被拒（BOT_QUOTA_EXCEEDED:maxSteps，不写库）", async ({ page }) => {
    await bootShim(page);
    await ipc(page, "set_bot_department_quota", {
      input: {
        departmentId: "dept-alpha", maxConcurrency: 1, totalTimeoutSecs: 60, maxSteps: 3,
        maxInFlightPerDepth: 2, stepTimeoutSecs: 300, maxConcurrentRuns: 2,
      },
    });

    const res = await ipc(page, "create_bot_workflow", {
      input: deptWorkflow({ steps: [step("s1"), step("s2"), step("s3"), step("s4")] }),
    });
    expect(res.ok).toBe(false);
    expect((res as any).error).toContain("BOT_QUOTA_EXCEEDED");
    expect((res as any).error).toContain("maxSteps");

    const state = await page.evaluate(() => (window as any).__P3_TEST__.state.workflows.length);
    expect(state).toBe(0);
  });

  test("AC-A5-2：设 2 步 → 3 步被拒；2 步通过", async ({ page }) => {
    await bootShim(page);
    await ipc(page, "set_bot_department_quota", {
      input: {
        departmentId: "dept-alpha", maxConcurrency: 2, totalTimeoutSecs: 900, maxSteps: 2,
        maxInFlightPerDepth: 2, stepTimeoutSecs: 300, maxConcurrentRuns: 2,
      },
    });

    const three = await ipc(page, "create_bot_workflow", {
      input: deptWorkflow({ steps: [step("s1"), step("s2"), step("s3")] }),
    });
    expect((three as any).error).toContain("BOT_QUOTA_EXCEEDED");

    const two = await ipc(page, "create_bot_workflow", {
      input: deptWorkflow({ steps: [step("s1"), step("s2")] }),
    });
    expect(two.ok).toBe(true);
    expect((two as any).value.steps).toHaveLength(2);
  });

  test("AC-A5-3：恢复默认后 3 步编排通过（reset → isDefault=true）", async ({ page }) => {
    await bootShim(page);
    await ipc(page, "set_bot_department_quota", {
      input: {
        departmentId: "dept-alpha", maxConcurrency: 2, totalTimeoutSecs: 900, maxSteps: 2,
        maxInFlightPerDepth: 2, stepTimeoutSecs: 300, maxConcurrentRuns: 2,
      },
    });
    await ipc(page, "reset_bot_department_quota", { departmentId: "dept-alpha" });

    const after = await ipc(page, "get_bot_department_quota", { departmentId: "dept-alpha" });
    expect((after as any).value).toMatchObject({ maxSteps: 12, isDefault: true, updatedAt: null });

    const three = await ipc(page, "create_bot_workflow", {
      input: deptWorkflow({ steps: [step("s1"), step("s2"), step("s3")] }),
    });
    expect(three.ok).toBe(true);
  });

  test("AC-A5-4：两部门配额互不影响（dept-alpha 2 步拒、dept-beta 3 步通过）", async ({ page }) => {
    await bootShim(page);
    await ipc(page, "set_bot_department_quota", {
      input: {
        departmentId: "dept-alpha", maxConcurrency: 2, totalTimeoutSecs: 900, maxSteps: 2,
        maxInFlightPerDepth: 2, stepTimeoutSecs: 300, maxConcurrentRuns: 2,
      },
    });

    const alpha = await ipc(page, "create_bot_workflow", {
      input: deptWorkflow({ steps: [step("s1"), step("s2"), step("s3")] }),
    });
    expect((alpha as any).error).toContain("BOT_QUOTA_EXCEEDED");

    const beta = await ipc(page, "create_bot_workflow", {
      input: deptWorkflow({
        departmentId: "dept-beta", steps: [step("s1"), step("s2"), step("s3")],
      }),
    });
    expect(beta.ok).toBe(true);

    const betaQuota = await ipc(page, "get_bot_department_quota", { departmentId: "dept-beta" });
    expect((betaQuota as any).value).toMatchObject({ maxSteps: 12, isDefault: true });
  });

  test("AC-A5-5（shim 侧）：部门在跑编排达 maxConcurrentRuns → 触发被拒且不新建 run", async ({ page }) => {
    await bootShim(page);
    await ipc(page, "set_bot_department_quota", {
      input: {
        departmentId: "dept-alpha", maxConcurrency: 1, totalTimeoutSecs: 60, maxSteps: 3,
        maxInFlightPerDepth: 2, stepTimeoutSecs: 300, maxConcurrentRuns: 1,
      },
    });
    const wf = await ipc(page, "create_bot_workflow", {
      input: deptWorkflow({ steps: [step("s1")] }),
    });
    expect(wf.ok).toBe(true);
    const workflowId = (wf as any).value.id;

    const first = await ipc(page, "run_bot_workflow", {
      workflowId, requesterConversationId: "thread-quota-1",
    });
    expect(first.ok).toBe(true);

    const second = await ipc(page, "run_bot_workflow", {
      workflowId, requesterConversationId: "thread-quota-2",
    });
    expect(second.ok).toBe(false);
    expect((second as any).error).toContain("BOT_QUOTA_EXCEEDED");
    expect((second as any).error).toContain("maxConcurrentRuns");

    const runs = await page.evaluate(() => (window as any).__P3_TEST__.state.runs.length);
    expect(runs).toBe(1);
  });

  test("ADR-P4 D2：ownerScope='instance' 的定义不受部门配额约束", async ({ page }) => {
    await bootShim(page);
    await ipc(page, "set_bot_department_quota", {
      input: {
        departmentId: "dept-alpha", maxConcurrency: 1, totalTimeoutSecs: 60, maxSteps: 1,
        maxInFlightPerDepth: 1, stepTimeoutSecs: 30, maxConcurrentRuns: 1,
      },
    });

    const instance = await ipc(page, "create_bot_workflow", {
      input: {
        name: "实例编排", ownerScope: "instance", departmentId: "dept-alpha",
        steps: [step("s1"), step("s2"), step("s3")],
      },
    });
    expect(instance.ok).toBe(true);

    // 同一 departmentId 但 instance 归属：运行也不吃部门并发额度
    const run = await ipc(page, "run_bot_workflow", {
      workflowId: (instance as any).value.id, requesterConversationId: "thread-instance",
    });
    expect(run.ok).toBe(true);
    expect(await countCalls(page, "run_bot_workflow")).toBe(1);
  });
});

// ── B. 页面证据（A4 未落地：fixme 显式待办） ────────────────────────────

test.describe("P4-a 配额 · 部门弹窗面板", () => {
  test("UI：打开部门配额面板，6 项默认值来自 get 且显示 isDefault 提示", async ({ page }) => {
    await page.addInitScript(installP3Shim, {
      bots: QUOTA_BOTS, departments: QUOTA_DEPARTMENTS,
    });
    await page.goto("/bots");
    await dismissOnboarding(page);
    await page.getByTestId("bot-department-quota-open").click();

    await expect(page.getByTestId("bot-quota-panel")).toBeVisible();
    await expect(page.getByTestId("bot-quota-max-concurrency")).toHaveValue("2");
    await expect(page.getByTestId("bot-quota-total-timeout")).toHaveValue("900");
    await expect(page.getByTestId("bot-quota-max-steps")).toHaveValue("12");
    await expect(page.getByTestId("bot-quota-max-in-flight")).toHaveValue("2");
    await expect(page.getByTestId("bot-quota-step-timeout")).toHaveValue("300");
    await expect(page.getByTestId("bot-quota-max-runs")).toHaveValue("2");
    await expect(page.getByTestId("bot-quota-default-hint")).toBeVisible();
  });

  // 2026-11-25 裁定（架构线）：前端预校验对用户展示**本地化字段标签**，不断言内部键名；
  // 且面板结构上不会把越界值提交给后端（预校验先拦），后端 `BOT_QUOTA_INVALID` 的
  // detail 透传改由组件测试钉住（BotP4Panels「后端码映射」用例），不在 e2e 里伪造旁路。
  test("UI：越界被前端预校验拦下，错误位显示本地化字段名且不提交", async ({ page }) => {
    await page.addInitScript(installP3Shim, {
      bots: QUOTA_BOTS, departments: QUOTA_DEPARTMENTS,
    });
    await page.goto("/bots");
    await dismissOnboarding(page);
    await page.getByTestId("bot-department-quota-open").click();

    await page.getByTestId("bot-quota-max-concurrency").fill("5");
    await page.getByTestId("bot-quota-save").click();
    const err = page.getByTestId("bot-quota-error");
    await expect(err).toContainText("取值超出允许范围");
    await expect(err).toContainText("单编排并发上限");
    // 预校验拦截：未发出 set_bot_department_quota
    expect(await countCalls(page, "set_bot_department_quota")).toBe(0);
  });

  test("UI：保存后回显 + 恢复默认二次确认后回到 isDefault 读数", async ({ page }) => {
    await page.addInitScript(installP3Shim, {
      bots: QUOTA_BOTS, departments: QUOTA_DEPARTMENTS,
    });
    await page.goto("/bots");
    await dismissOnboarding(page);
    await page.getByTestId("bot-department-quota-open").click();

    await page.getByTestId("bot-quota-max-steps").fill("3");
    await page.getByTestId("bot-quota-save").click();
    await expect(page.getByTestId("bot-quota-max-steps")).toHaveValue("3");
    await expect(page.getByTestId("bot-quota-default-hint")).toBeHidden();

    await page.getByTestId("bot-quota-reset").click();
    await page.locator(".ant-popconfirm-buttons .ant-btn-primary").click();
    await expect(page.getByTestId("bot-quota-max-steps")).toHaveValue("12");
    await expect(page.getByTestId("bot-quota-default-hint")).toBeVisible();
  });
});