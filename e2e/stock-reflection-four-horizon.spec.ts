// SPDX-License-Identifier: AGPL-3.0-only
//
// 批次 5（`PLAN-stock-reflection-four-horizon.md` 阶段 E）：
// 用**固定 fixture** 验证四周期反思面板的四条放行条件 ——
//   ① 周期状态可区分：已成熟 / 未到期 / 无数据 / 旧记录；
//   ② 未成熟（`wasCorrect: null`）与行情不可用**不得**被判成「正确 / 错误」；
//   ③ `null` 指标不得显示为 `0`（收益 None → 「—」，命中率样本不足 → 「样本不足」）；
//   ④ 切换周期不得串用其它周期的决策方向与收益。
//
// 数据通路：`list_reflections` 在浏览器模式下读 localStorage `axagent_mock.reflections`
// （见 `src/lib/browserMock.ts` 的同名 case），本文件用 `addInitScript` 预置 fixture。
// **不预置该 key 时 mock 返回 `[]`**，其余 e2e 与本用例互相隔离。
//
// ⚠ 断言必须限定在**展开行内**的激活面板：页面自身的 Tab 也是 `role="tabpanel"`，
//   且 antd 会把切走的周期面板留在 DOM 里（`aria-hidden="true"`），
//   直接对整页取文本会让「不串线」类断言恒通过。

import { expect, type Locator, type Page, test } from "@playwright/test";

const FIXTURE_KEY = "axagent_mock.reflections";

/** 四周期全部成熟，四个方向的决策与收益**各不相同** —— 用于检出「串线」 */
const ROW_ALL_MATURE = {
  id: "ref-all-mature",
  stockCode: "600519",
  stockName: "贵州茅台",
  asOfDate: "2026-08-01",
  hindsightDate: "2026-09-01",
  actualOutcome: "方向正确",
  whatWentWrong: null,
  missedSignals: null,
  fixForFuture: null,
  reflectionDepth: "light",
  minConfidenceThreshold: 60,
  status: "completed",
  createdAt: 1_750_000_000_000,
  horizonResults: {
    ultra_short: {
      status: "mature",
      expectedHoldingDays: 3,
      actualHoldingDays: 3,
      decision: { action: "ULTRA_BUY", confidence: 0.7, positionPct: 0.1 },
      market: { returnPct: 3.5, alphaPct: 1.2 },
      evaluation: { wasCorrect: 1, directionMatch: true, targetHit: true },
    },
    short: {
      status: "mature",
      expectedHoldingDays: 5,
      actualHoldingDays: 5,
      decision: { action: "SHORT_SELL", confidence: 0.6, positionPct: 0 },
      market: { returnPct: -2.25, alphaPct: -0.8 },
      evaluation: { wasCorrect: 0, directionMatch: false, targetHit: false },
    },
    mid: {
      status: "mature",
      expectedHoldingDays: 28,
      actualHoldingDays: 28,
      decision: { action: "MID_HOLD", confidence: 0.5, positionPct: 0.2 },
      market: { returnPct: 1.0, alphaPct: 0.3 },
      evaluation: { wasCorrect: 1, directionMatch: true, targetHit: null },
    },
    long: {
      status: "mature",
      expectedHoldingDays: 90,
      actualHoldingDays: 90,
      decision: { action: "LONG_ACCUMULATE", confidence: 0.8, positionPct: 0.35 },
      market: { returnPct: 12.75, alphaPct: 6.5 },
      evaluation: { wasCorrect: 1, directionMatch: true, targetHit: true },
    },
  },
};

/** 部分成熟：超短成熟 / 短线未到期 / 中线行情不可用 / 长线无记录 */
const ROW_PARTIAL = {
  id: "ref-partial",
  stockCode: "000001",
  stockName: "平安银行",
  asOfDate: "2026-08-20",
  hindsightDate: "2026-09-10",
  actualOutcome: "待观察",
  whatWentWrong: null,
  missedSignals: null,
  fixForFuture: null,
  reflectionDepth: "light",
  minConfidenceThreshold: 60,
  status: "completed",
  createdAt: 1_750_100_000_000,
  horizonResults: {
    ultra_short: {
      status: "mature",
      expectedHoldingDays: 3,
      actualHoldingDays: 3,
      decision: { action: "PARTIAL_ULTRA_BUY", confidence: 0.7 },
      market: { returnPct: 4.25 },
      evaluation: { wasCorrect: 1, directionMatch: true, targetHit: null },
    },
    short: {
      status: "immature",
      expectedHoldingDays: 5,
      actualHoldingDays: 2,
      decision: { action: "PARTIAL_SHORT_BUY", confidence: 0.65 },
      // 未到期 ⇒ 行情与判定均缺失，且**不得**渲染成 0
      market: { returnPct: null, alphaPct: null, targetReached: null, stopLossTriggered: null },
      evaluation: { wasCorrect: null, directionMatch: null, targetHit: null },
    },
    mid: {
      status: "unavailable",
      expectedHoldingDays: 28,
      actualHoldingDays: null,
      decision: { action: "PARTIAL_MID_HOLD", confidence: 0.4 },
      market: null,
      evaluation: { wasCorrect: null, directionMatch: null, targetHit: null },
    },
  },
};

/** 旧记录：四周期 JSON 里只有 legacy 主周期 */
const ROW_LEGACY = {
  id: "ref-legacy",
  stockCode: "300750",
  stockName: "宁德时代",
  asOfDate: "2025-12-01",
  hindsightDate: "2026-01-15",
  actualOutcome: "方向错误",
  whatWentWrong: "估值高位未减仓",
  missedSignals: null,
  fixForFuture: null,
  reflectionDepth: "light",
  minConfidenceThreshold: 60,
  status: "completed",
  createdAt: 1_740_000_000_000,
  horizonResults: {
    long: {
      status: "legacy",
      expectedHoldingDays: 90,
      decision: { action: "LEGACY_LONG_BUY", confidence: 0.6 },
      market: { returnPct: 8.0 },
      evaluation: { wasCorrect: 0, directionMatch: false, targetHit: false },
    },
  },
};

type FixtureRow = typeof ROW_ALL_MATURE | typeof ROW_PARTIAL | typeof ROW_LEGACY;

async function seedFixture(page: Page, rows: FixtureRow[]): Promise<void> {
  await page.addInitScript(
    ({ key, data }) => {
      try {
        // 预置 onboarding 完成，避免 WelcomeWizard 遮挡页面
        localStorage.setItem(
          "axagent_settings",
          JSON.stringify({
            onboardingCompleted: true,
            onboardingWizardDismissed: true,
            onboardingTutorialCompleted: true,
          }),
        );
        localStorage.setItem(key, JSON.stringify(data));
      } catch {
        /* ignore */
      }
    },
    { key: FIXTURE_KEY, data: rows },
  );
}

/**
 * 进入「金融投资 → 工作区 → 复盘」视图 —— `ReviewView` 默认就是「反思」子视图，
 * 直接渲染 `ReflectionPanel`（无需再点 Tab）。
 *
 * 路径依据：`/invest?tab=workspace&view=review`（view 键取自
 * `StockWorkspaceShell` 的 DESKTOP_TABS）。曾经的 `/stock-analysis/:id` 只是
 * 重定向入口，且 `view=analysis` 走的是分析视图而非反思面板。
 */
async function openReflectionPanel(page: Page): Promise<void> {
  await page.goto("/invest?tab=workspace&view=review");
  await page.waitForLoadState("domcontentloaded");

  // 关掉可能存在的引导弹窗（与 workflow-editor.spec.ts 同法）
  const skip = page.getByTestId("onboarding-skip").first();
  if (await skip.isVisible({ timeout: 1000 }).catch(() => false)) {
    await skip.click({ force: true });
  }

  // 反思历史表格出现 ⇒ 面板与 fixture 数据都已就绪
  await expect(page.locator(".ant-table-tbody tr.ant-table-row").first()).toBeVisible({
    timeout: 20000,
  });
}

/** 展开第 index 行，返回该展开行（后续断言一律限定其内） */
async function expandRow(page: Page, index: number) {
  const dataRow = page.locator(".ant-table-tbody tr.ant-table-row").nth(index);
  await expect(dataRow).toBeVisible({ timeout: 15000 });
  await dataRow.locator(".ant-table-row-expand-icon").click();

  const expanded = page.locator("tr.ant-table-expanded-row");
  await expect(expanded).toHaveCount(1, { timeout: 10000 });
  await expect(expanded).toBeVisible();
  return expanded;
}

/** 展开行内**当前激活**的周期面板 */
function activePane(expanded: Locator): Locator {
  return expanded.locator('[role="tabpanel"][aria-hidden="false"]');
}

function horizonTab(expanded: Locator, labelPrefix: string): Locator {
  return expanded.locator(".ant-tabs-tab", { hasText: labelPrefix });
}

test.describe("四周期反思面板（批次 5 fixture 验收）", () => {
  test("四周期全成熟：切换周期只显示该周期自己的方向与收益（不串线）", async ({ page }) => {
    await seedFixture(page, [ROW_ALL_MATURE]);
    await openReflectionPanel(page);
    const expanded = await expandRow(page, 0);

    // 四个成熟周期各自生成 Tab
    await expect(expanded.locator(".ant-tabs-tab")).toHaveCount(4);

    // 默认激活首周期（超短线）
    await expect(activePane(expanded)).toContainText("ULTRA_BUY");
    await expect(activePane(expanded)).toContainText("+3.50%");
    await expect(activePane(expanded)).toContainText("✓ 正确");

    // 切到短线：显示自己的方向/收益，不得带上超短线的值
    await horizonTab(expanded, "短线 (5天)").click();
    await expect(activePane(expanded)).toContainText("SHORT_SELL");
    await expect(activePane(expanded)).toContainText("-2.25%");
    await expect(activePane(expanded)).toContainText("✗ 错误");
    await expect(activePane(expanded)).not.toContainText("ULTRA_BUY");
    await expect(activePane(expanded)).not.toContainText("+3.50%");

    // 切到长线：同样不得串用中线/短线
    await horizonTab(expanded, "长线 (90+天)").click();
    await expect(activePane(expanded)).toContainText("LONG_ACCUMULATE");
    await expect(activePane(expanded)).toContainText("+12.75%");
    await expect(activePane(expanded)).not.toContainText("MID_HOLD");
  });

  test("部分成熟：未到期与无数据不被判成「正确/错误」，且 null 不显示为 0", async ({ page }) => {
    await seedFixture(page, [ROW_PARTIAL]);
    await openReflectionPanel(page);
    const expanded = await expandRow(page, 0);

    // 长线无记录 ⇒ 只生成 3 个 Tab，且不出现「该周期无记录」占位
    await expect(expanded.locator(".ant-tabs-tab")).toHaveCount(3);
    await expect(expanded).not.toContainText("该周期无记录");

    // 超短线成熟 → 有判定
    await expect(activePane(expanded)).toContainText("✓ 正确");

    // 短线未到期
    await horizonTab(expanded, "短线 (5天)").click();
    await expect(activePane(expanded)).toContainText("未到期");
    await expect(activePane(expanded)).toContainText("行情尚未到达期望持有期");
    await expect(activePane(expanded)).not.toContainText("✓ 正确");
    await expect(activePane(expanded)).not.toContainText("✗ 错误");
    // null 指标：涨跌幅必须是「—」，不得伪报 0
    await expect(activePane(expanded)).toContainText("—");
    await expect(activePane(expanded)).not.toContainText("0.00%");

    // 中线行情不可用
    await horizonTab(expanded, "中线 (28天)").click();
    await expect(activePane(expanded)).toContainText("无数据");
    await expect(activePane(expanded)).toContainText("该周期行情数据不可用");
    await expect(activePane(expanded)).not.toContainText("行情事实");
    await expect(activePane(expanded)).not.toContainText("✓ 正确");

    // 全页不得出现「样本不足被写成 0.0%」这类伪报命中率
    await expect(page.locator("body")).not.toContainText("0.0%");
  });

  test("旧记录：显示「旧记录」状态与说明，不与其它状态文案混淆", async ({ page }) => {
    await seedFixture(page, [ROW_LEGACY]);
    await openReflectionPanel(page);
    const expanded = await expandRow(page, 0);

    await expect(expanded.locator(".ant-tabs-tab")).toHaveCount(1);
    await expect(activePane(expanded)).toContainText("旧记录");
    await expect(activePane(expanded)).toContainText("此为四周期功能上线前的单周期旧记录");
    await expect(activePane(expanded)).toContainText("LEGACY_LONG_BUY");
    await expect(activePane(expanded)).not.toContainText("未到期");
    await expect(activePane(expanded)).not.toContainText("行情尚未到达期望持有期");
  });
});
