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
    decisionsByHorizon?:
      | Record<
        string,
        Partial<import("@/types").HorizonDecision> & { action: string }
      >
      | null;
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

  // ── 2026-10-04 R-11：逐档面板的两条呈现判据（口径 A 的展示侧）──────────────
  // 原两条用例锁的是「乘数降权注脚挂在哪一档」；乘数表已退役 ⇒ 注脚连主语都没了。
  // 换成本片真实改变的两件事：
  //   ① 该档技术腿退化必须**成句**（不得退化成「评分低」或干脆不显示）；
  //   ② 某路分支没产出 ⇒ 该档整个 Tab 消失，且不得冒出任何「该档结论」占位文案。
  it("该档技术腿按日线退化 ⇒ 注脚必须成句出现", () => {
    storeState.decision = {
      action: "BUY",
      positionPct: 10.0,
      reasoning: "技术面突破",
      riskLevel: "中",
      confidence: 0.8,
      decisionsByHorizon: {
        ultraShort: { action: "BUY", positionPct: 10, confidence: 60 },
        // 中线档的逐档粒度评分没出数 ⇒ f1 腿退回日线（结构性缺席，不是低分）
        mid: { action: "HOLD", positionPct: 5, confidence: 55, scoreSource: "daily_fallback" },
      },
    };
    storeState.stockCode = "600519";
    render(
      <MemoryRouter>
        <DecisionBanner />
      </MemoryRouter>,
    );
    // 默认选中第一档（ultraShort）⇒ 退化注脚不该出现在这里
    expect(screen.queryByText("stockAnalysis.horizonScoreFallbackHint")).toBeNull();
    // 点到中线档才出现：注脚是**逐档**的，不是全局横幅
    fireEvent.click(screen.getByText("stockAnalysis.timeHorizonMid"));
    expect(screen.getByText("stockAnalysis.horizonScoreFallbackHint")).toBeTruthy();
  });

  it("逐档「本档实际几根」成句上屏：有回显就印尺度+窗口，没有回显就不渲染该行（v138 裁定 3）", () => {
    // 为什么这条要进门：v136 起四档各自按尺度取数、按该档窗口计划出指标，但界面只有分数 ⇒
    // 「这一档算得粗」与「这一档观点不同」同形。短/中/长三档的根数按公式本就是同一组
    // （差别在 2 周 / 2 月 / 2 季）⇒ 只有把**根数与尺度并列**才读得出来。
    // 断言面分两层：① 传进去的必须是该档自己的回显值（不是日线那组、不是常量）；
    //                ② 旧代行（无回显）不得被渲染成 0 或空表 —— 那是把「没有这一项」伪装成「值为 0」。
    storeState.decision = {
      action: "BUY",
      positionPct: 10.0,
      reasoning: "技术面突破",
      riskLevel: "中",
      confidence: 0.8,
      decisionsByHorizon: {
        // 超短档：早于 v138 的存量行 —— 没有窗口回显
        ultraShort: { action: "BUY", positionPct: 10, confidence: 60 },
        // 中线档：按月线 + 该档窗口计划算出来的那组（产端 = IndicatorWindows 回显）
        mid: {
          action: "HOLD",
          positionPct: 5,
          confidence: 55,
          scoringScale: "monthly",
          scoringWindows: {
            maPeriods: [2, 6],
            macdFast: 2,
            macdSlow: 3,
            macdSignal: 2,
            rsiPeriods: [2],
            bollPeriod: 2,
            volumeLookback: 2,
          },
          // v139：两带数值 + **diffPct / fastSlope 给 null**（= 慢带非正、上一根不可算），
          // 检验呈现层不把「算不出」压成 0。
          scoringTrend: { fastBars: 2, slowBars: 6, fast: 11.5, slow: 10.25, diffPct: null, fastSlope: null },
          scoringMomentum: { period: 2, value: 44 },
        },
        // 短线档：两带数值齐全且与中线那份夹具**不同** ⇒ 锁的是「逐档各自的数」而不是同一份常量。
        short: {
          action: "BUY",
          positionPct: 4,
          confidence: 61,
          scoringScale: "weekly",
          scoringWindows: {
            maPeriods: [2, 6],
            macdFast: 2,
            macdSlow: 3,
            macdSignal: 2,
            rsiPeriods: [2],
            bollPeriod: 2,
            volumeLookback: 2,
          },
          scoringTrend: { fastBars: 2, slowBars: 6, fast: 9.75, slow: 9.5, diffPct: 2.63, fastSlope: 0.11 },
          scoringMomentum: { period: 2, value: 51.5 },
        },
      },
    };
    storeState.stockCode = "600519";
    render(
      <MemoryRouter>
        <DecisionBanner />
      </MemoryRouter>,
    );
    // 默认档（超短）是旧代行 ⇒ 整行不渲染，且不得出现「MA 0」「0/0」这类补零形态
    expect(screen.queryByText(/stockAnalysis\.horizonScoringWindows/)).toBeNull();
    fireEvent.click(screen.getByText("stockAnalysis.timeHorizonMid"));
    expect(
      screen.getByText(
        "stockAnalysis.horizonScoringWindows|scale=monthly,ma=2/6,rsi=2,macd=2/3/2,boll=2,vol=2",
      ),
    ).toBeTruthy();
    // v139 两带数值成句，且 null 必须是「—」而不是 0
    expect(
      screen.getByText(
        "stockAnalysis.horizonScaleTrend|fastBars=2,slowBars=6,fast=11.50,slow=10.25,diffPct=—,fastSlope=—",
      ),
    ).toBeTruthy();
    expect(screen.getByText("stockAnalysis.horizonScaleMomentum|period=2,value=44.0")).toBeTruthy();
    // 换档 ⇒ 数值跟着换（若组件读的是同一份常量，这条会红）
    fireEvent.click(screen.getByText("stockAnalysis.timeHorizonShort"));
    expect(
      screen.getByText(
        "stockAnalysis.horizonScaleTrend|fastBars=2,slowBars=6,fast=9.75,slow=9.50,diffPct=2.63,fastSlope=0.11",
      ),
    ).toBeTruthy();
    expect(screen.getByText("stockAnalysis.horizonScaleMomentum|period=2,value=51.5")).toBeTruthy();
  });

  it("某路分支未产出 ⇒ 该档 Tab 整个消失（显式缺席，不补占位行）", () => {
    storeState.decision = {
      action: "BUY",
      positionPct: 10.0,
      reasoning: "技术面突破",
      riskLevel: "中",
      confidence: 0.8,
      // 四路只有中线一路有结论（其余三路节点失败/未接线）
      decisionsByHorizon: { mid: { action: "HOLD", positionPct: 5, confidence: 55 } },
    };
    storeState.stockCode = "600519";
    const { container } = render(
      <MemoryRouter>
        <DecisionBanner />
      </MemoryRouter>,
    );
    expect(screen.getByText("stockAnalysis.timeHorizonMid")).toBeTruthy();
    // 缺席档在**任何**呈现面上都不出现：Tab 是独立文本节点，而概览条把标签和动作拼进
    // 同一个节点（`中线: 观望`）⇒ 只按精确文本 queryByText 会漏掉概览条那一族（假绿方向
    // 正是「缺档被补了一行」），故这里按整棵子树的 textContent 判。
    const text = container.textContent ?? "";
    for (const gone of ["UltraShort", "Short", "Long"]) {
      expect(text.includes(`stockAnalysis.timeHorizon${gone}`)).toBe(false);
    }
    // Tab + 概览条两处 = 2 次；多一次就说明某处又给缺档补了行
    expect((text.match(/stockAnalysis\.timeHorizonMid/g) ?? []).length).toBe(2);
  });

  // ── 阶段1（PLAN §四十八 Q3-A）：逐档证据必须上屏 ─────────────────────────────
  // 四档结论不同是**腿集与算法**不同（R-11）造成的；只给四个 action 就等于把「为什么不同」
  // 留在后端，读者只能把差异读成噪声。这里锁的是「分支自证字段每一项都有落点」。
  it("逐档证据上屏：腿/缺腿/门与判定依据/出场与置信口径/先验来源与样本数", () => {
    storeState.decision = {
      action: "BUY",
      positionPct: 10.0,
      reasoning: "技术面突破",
      riskLevel: "中",
      confidence: 0.8,
      decisionsByHorizon: {
        mid: {
          action: "持有",
          positionPct: 0,
          confidence: 56.8,
          odds: 0,
          entryGate: "inside_sigma_band",
          entryGatePassed: false,
          gateBasis: "unjudged",
          exitRule: "k_sigma_band",
          confidenceMethod: "sigma_band_position",
          priorSource: "pooled",
          priorSamples: 0,
          evidenceScale: 20,
          legs: [
            { factor: "momentumSignal", role: "direction", weight: 0.4, signal: 0.7 },
            { factor: "supplyShock", role: "riskNote", weight: 0, signal: 0.2 },
          ],
          absentLegs: ["expectationRevision", "sectorRotation"],
          dataGaps: ["mid 档 expectationRevision 腿本轮不可评估"],
        },
      },
    };
    storeState.stockCode = "600519";
    const { container } = render(
      <MemoryRouter>
        <DecisionBanner />
      </MemoryRouter>,
    );
    const text = container.textContent ?? "";
    for (
      const must of [
        "stockAnalysis.horizonEntryGateLabel",
        "inside_sigma_band",
        // 门「算不出」必须与「未通过」分列（把未判定读成未通过 = 另一种伪装成结论）
        "stockAnalysis.horizonGateUnjudged",
        "stockAnalysis.horizonExitRuleLabel",
        "k_sigma_band",
        "stockAnalysis.horizonConfidenceMethodLabel",
        "sigma_band_position",
        "stockAnalysis.horizonOddsLabel",
        "stockAnalysis.horizonEvidenceScaleLabel",
        "stockAnalysis.horizonLegsLabel|count=2",
        "momentumSignal",
        "stockAnalysis.horizonAbsentLegsLabel|count=2",
        "expectationRevision",
        "stockAnalysis.horizonTierGapsLabel|count=1",
        // 先验样本为 0 ⇒ 必须成句说明「借自全档合并基准」
        "stockAnalysis.horizonPriorNoSamples",
      ]
    ) {
      expect(text, `逐档面板缺呈现项: ${must}`).toContain(must);
    }
    expect(text).toContain("stockAnalysis.horizonPriorLabel|source=pooled,n=0");
  });

  // ── 阶段1（PLAN §四十八 Q2-B）：方向成立但无可执行计划 ⇒ 保留方向 + 并列成句 ──
  it("方向族 + 仓位 0 + 赔率 0 ⇒ 必须并列声明「无可执行计划」；观望档不声明（0 仓位是它的常态）", () => {
    storeState.decision = {
      action: "BUY",
      positionPct: 10.0,
      reasoning: "技术面突破",
      riskLevel: "中",
      confidence: 0.8,
      decisionsByHorizon: {
        ultraShort: { action: "买入", positionPct: 0, confidence: 87.8, odds: 0 },
        short: { action: "观望", positionPct: 0, confidence: 40, odds: 0 },
      },
    };
    storeState.stockCode = "600519";
    render(
      <MemoryRouter>
        <DecisionBanner />
      </MemoryRouter>,
    );
    // 默认选中第一档（ultraShort，买入 + 0 仓位）⇒ 声明必须在
    expect(screen.getByText("stockAnalysis.horizonNoExecutablePlan")).toBeTruthy();
    // 切到观望档 ⇒ 同一声明不得出现（0 仓位对观望是正常态，声明它就是把常态说成缺陷）
    fireEvent.click(screen.getByText("stockAnalysis.timeHorizonShort"));
    expect(screen.queryByText("stockAnalysis.horizonNoExecutablePlan")).toBeNull();
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

/**
 * 主档来历行（v127，PLAN §五十三 ①）。
 *
 * 被测的不是「有没有一段文字」，而是两条互斥义务：
 *   ① 有 `actionSource` 且属于降级格 ⇒ 必须成句说出「哪一档、分支原结论、被谁改写、现在是什么」
 *      （000710 的形态：超短线分支=买入 → 高风险风控否决 → 主档=持有）；
 *   ② 没有该字段（v127 之前的存量行）⇒ **整行不渲染**，
 *      不得回退成「分支选档」—— 那是给旧记录编一个它没有的来历。
 */
describe("DecisionBanner 主档来历行", () => {
  const renderBanner = () => {
    const { container } = render(
      <MemoryRouter>
        <DecisionBanner />
      </MemoryRouter>,
    );
    return container.textContent ?? "";
  };

  it("降级格：成句必须带分支原结论与改写者", () => {
    storeState.decision = {
      action: "HOLD",
      positionPct: 20.3,
      reasoning: "决策=持有",
      riskLevel: "高",
      confidence: 61,
      actionSource: "risk_veto_downgrade",
      confidenceSource: "branch_row",
      timeHorizon: "ultra_short",
      decisionsByHorizon: { ultraShort: { action: "买入" }, short: { action: "持有" } },
    } as typeof storeState.decision;
    const text = renderBanner();
    expect(text).toContain("stockAnalysis.decisionProvenanceDowngraded");
    expect(text).toContain("reason=stockAnalysis.actionSourceRiskVeto");
    expect(text).toContain("branchAction=");
    // 置信口径是 branch_row ⇒ 不该再多说一句（只有退回主链时才需要点名）
    expect(text).not.toContain("stockAnalysis.confidenceSourceMainChain");
  });

  it("四路全缺席那代：置信口径退回主链时必须点名", () => {
    storeState.decision = {
      action: "HOLD",
      positionPct: 8,
      reasoning: "决策=持有",
      riskLevel: "中",
      confidence: 44,
      actionSource: "main_chain",
      confidenceSource: "main_chain_posterior",
      timeHorizon: "short",
    } as typeof storeState.decision;
    const text = renderBanner();
    expect(text).toContain("stockAnalysis.actionSourceMainChain");
    expect(text).toContain("stockAnalysis.confidenceSourceMainChain");
    expect(text).not.toContain("decisionProvenanceDowngraded");
  });

  it("v127 之前的存量行：来历整行不渲染（缺席不编造）", () => {
    storeState.decision = {
      action: "BUY",
      positionPct: 10,
      reasoning: "技术面突破",
      riskLevel: "中",
      confidence: 0.8,
    } as typeof storeState.decision;
    const text = renderBanner();
    expect(text).not.toContain("decisionProvenance");
    expect(text).not.toContain("actionSource");
  });
});
