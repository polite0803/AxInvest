import { fireEvent, render, screen } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { describe, expect, it, vi } from "vitest";
import { DecisionBanner } from "../DecisionBanner";

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    // 原实现对第二参数一律当「字符串兜底」返回 ⇒ 遇到 `t(key, { good, total, avg })`
    // 这种**插值对象**会把整个对象当 React child 渲染，抛
    // "Objects are not valid as a React child (found: object with keys {good, total, avg})"
    // —— 那是**测试替身**的缺陷，不是被测组件的缺陷（2026-09-21 实测踩到）。
    // 现按形态分流：字符串 ⇒ 兜底原文；对象 ⇒ 拼成 `key|k=v,k=v`（便于断言「传了哪个值」）。
    t: (key: string, second?: unknown) => {
      if (typeof second === "string") { return second; }
      if (second && typeof second === "object") {
        const kv = Object.entries(second as Record<string, unknown>)
          .map(([k, v]) => `${k}=${v}`)
          .join(",");
        return `${key}|${kv}`;
      }
      return key;
    },
  }),
  initReactI18next: { type: "3rdParty", init: () => {} },
}));

// Default mock: no decision
const storeState = {
  decision: null as {
    action: string;
    positionPct: number;
    reasoning: string;
    riskLevel: string;
    confidence: number;
    targetPrice?: number;
    stopLoss?: number;
  } | null,
  stockCode: "600519" as string | null,
  stockName: "茅台",
  startAnalysis: vi.fn(),
  // data-quality 节点输出的 JSON 字符串。2026-09-21 新增：逐节点表格的「报告质量」列
  //   读的是这里面的 diagnostics[*].report_quality。默认空串 ⇒ 不渲染数据质量区块，
  //   既有 4 个用例的行为不受影响。
  dataQualitySummary: "" as string,
};

vi.mock("@/stores", () => ({
  useStockAnalysisStore: (selector: (s: typeof storeState) => unknown) => selector(storeState),
  useSettingsStore: (selector: (s: { settings: { theme_mode: string } }) => unknown) =>
    selector({ settings: { theme_mode: "system" } }),
}));

describe("DecisionBanner", () => {
  it("decision 为 null 时渲染'决策缺失'占位卡（不再 firstChild === null）", () => {
    storeState.decision = null;
    storeState.stockCode = "600519";
    const { container } = render(
      <MemoryRouter>
        <DecisionBanner />
      </MemoryRouter>,
    );
    expect(container.firstChild).not.toBeNull();
    expect(screen.getByTestId("decision-banner-missing")).toBeTruthy();
    expect(container.textContent).toContain("stockAnalysis.decisionMissing");
    expect(container.textContent).toContain("stockAnalysis.decisionMissingHint");
  });

  it("占位卡有 stockCode 时显示'重跑分析'按钮", () => {
    storeState.decision = null;
    storeState.stockCode = "600519";
    const { container } = render(
      <MemoryRouter>
        <DecisionBanner />
      </MemoryRouter>,
    );
    expect(screen.getByTestId("decision-banner-missing")).toBeTruthy();
    expect(container.textContent).toContain("stockAnalysis.reAnalyze");
  });

  it("占位卡无 stockCode 时显示'搜索股票'按钮(永远有入口)", () => {
    storeState.decision = null;
    storeState.stockCode = null;
    const { container } = render(
      <MemoryRouter>
        <DecisionBanner />
      </MemoryRouter>,
    );
    expect(screen.getByTestId("decision-banner-missing")).toBeTruthy();
    // 永远有按钮（不重跑就跳到搜索栏），不出现 dead-end
    expect(container.textContent).toContain("stockAnalysis.searchStock");
    expect(container.textContent).toContain("stockAnalysis.reAnalyzeNeedCodeHint");
  });

  it("renders decision info when decision exists", () => {
    storeState.decision = {
      action: "BUY",
      positionPct: 10.0,
      reasoning: "技术面突破",
      riskLevel: "中",
      confidence: 0.8,
      targetPrice: 1850.0,
      stopLoss: 1580.0,
    };
    storeState.stockCode = "600519";
    const { container } = render(
      <MemoryRouter>
        <DecisionBanner />
      </MemoryRouter>,
    );
    expect(container.firstChild).not.toBeNull();
    expect(container.textContent).toContain("stockAnalysis.actionBuy");
    expect(container.textContent).toContain("10%");
  });
});

// ── 2026-09-21 新增：数据质量逐节点表格的「报告质量」列 ──────────────────────
//
// 背景（用户质问「所有分析师节点的数据质量监控都是这个结论，这是造假吗」）：
//   区块顶部的 grade / score / good_count 取自 data-quality 节点的**全局聚合**输出，
//   10 张分析师卡片打开后看到同一组数字（`AnalystDataQualityModal` 同源问题已另行修）。
//   本次给逐节点表格补一列**本节点自己的** `report_quality`（事实量，非等级），
//   使「全局聚合」与「单节点事实」能在同一屏内被区分开。
//
// ⚠ 断言必须**定位到单元格**，不能用 `container.textContent).toContain("88")` ——
//   后者只要页面上任何位置出现该数字就通过（例如同行的置信度正好也是 88），
//   无法证明它落在「报告质量」列上。这正是「拿读数当结论」的形态。

/** 列序：0 分析师 / 1 预期数据 / 2 置信度 / 3 失败标记 / 4 报告质量 / 5 状态 / 6 差距原因 */
const RQ_COL_INDEX = 4;

function buildDqSummary(
  rows: Array<{ key: string; name: string; rq?: number; conf?: number }>,
): string {
  const diagnostics: Record<string, unknown> = {};
  for (const r of rows) {
    const item: Record<string, unknown> = {
      name: r.name,
      status: "normal",
      confidence: r.conf ?? 72,
      expected_data: "expected",
      gap_reason: "",
      placeholder_hits: 0,
    };
    // 只在给了 rq 时才写入该字段 —— 用于模拟「旧版快照没有 report_quality」
    if (r.rq !== undefined) { item.report_quality = r.rq; }
    diagnostics[r.key] = item;
  }
  // grade 取 D：该区块的 Collapse 只有 D/F 才默认展开
  return JSON.stringify({
    grade: "D",
    score: 61.5,
    good_count: 6,
    total_analysts: 10,
    avg_confidence: 61.5,
    diagnostics,
  });
}

/**
 * 打开「完整详情」Modal。
 *
 * ⚠ 数据质量逐节点表格位于**该 Modal 内部**（`open={expanded}`，`expanded` 初值 false），
 *   且 antd Modal 走 portal 挂到 `document.body` ——
 *   ① 不点开就**根本不渲染**；② 渲染了也**不在** `render()` 返回的 container 里。
 *   本用例第一版正是因此在 container 里查 `thead th` 恒得空数组（4 个用例全红），
 *   而断言读起来像「列没加进去」—— 又一次「测试自身缺陷冒充被测对象缺陷」。
 */
function openDetailModal() {
  fireEvent.click(screen.getAllByRole("button", { name: /showDetail/ })[0]);
}

/** 按表头特征定位 Modal 内的数据质量逐节点表格（不依赖 DOM 顺序，避免串到别的表格） */
function dqTableOf(): HTMLTableElement {
  const hit = Array.from(document.body.querySelectorAll("table")).find((el) =>
    el.textContent?.includes("stockAnalysis.dqTableAnalyst")
  );
  if (!hit) { throw new Error("未找到数据质量逐节点表格 —— 详情 Modal 是否已打开？"); }
  return hit as HTMLTableElement;
}

/** 取表格每行「报告质量」列的单元格文本（按行序） */
function rqCellsOf(): string[] {
  return Array.from(dqTableOf().querySelectorAll("tbody tr")).map(
    (tr) => tr.children[RQ_COL_INDEX]?.textContent?.trim() ?? "",
  );
}

function renderWithDq(summary: string) {
  storeState.decision = {
    action: "BUY",
    positionPct: 10,
    reasoning: "技术面突破",
    riskLevel: "中",
    confidence: 0.8,
  };
  storeState.stockCode = "600519";
  storeState.dataQualitySummary = summary;
  render(
    <MemoryRouter>
      <DecisionBanner />
    </MemoryRouter>,
  );
  openDetailModal();
}

describe("DecisionBanner 数据质量表格的「报告质量」列（2026-09-21）", () => {
  it("表头含该列，且位置紧跟「失败标记」", () => {
    renderWithDq(buildDqSummary([{ key: "mk", name: "技术面", rq: 88 }]));
    const heads = Array.from(dqTableOf().querySelectorAll("thead th")).map((th) => th.textContent?.trim());
    expect(heads).toEqual([
      "stockAnalysis.dqTableAnalyst",
      "stockAnalysis.dqTableExpectedData",
      "stockAnalysis.dqTableConfidence",
      "stockAnalysis.dqTablePlaceholder",
      "stockAnalysis.dqTableReportQuality",
      "stockAnalysis.dqTableStatus",
      "stockAnalysis.dqTableGapReason",
    ]);
  });

  it("每行显示该节点**自己的** report_quality（两行值不同，不是全局均值）", () => {
    renderWithDq(buildDqSummary([
      { key: "mk", name: "技术面", rq: 88 },
      { key: "hm", name: "资金面", rq: 42 },
    ]));
    expect(rqCellsOf()).toEqual(["88", "42"]);
    // 反向断言：全局 score（61.5）不得出现在该列 —— 那正是「所有节点同数」的旧行为
    expect(rqCellsOf()).not.toContain("61.5");
  });

  it("0 是合法值（该节点未注入报告），显示 0 而不是占位符", () => {
    renderWithDq(buildDqSummary([{ key: "mk", name: "技术面", rq: 0 }]));
    expect(rqCellsOf()).toEqual(["0"]);
  });

  it("旧快照缺 report_quality ⇒ 显示「—」（既不能当 0，也不能是空字符串）", () => {
    renderWithDq(buildDqSummary([{ key: "mk", name: "技术面" }]));
    expect(rqCellsOf()).toEqual(["—"]);
  });
});
