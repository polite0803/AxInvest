// 估值评估 tab「页面错误」复现 + 回归测试：
// 用 DB 实际 value-investor 输出渲染 ValueAssessmentPanel，并锁定 ReportMarkdown 的 content 收敛契约
import { render, screen, waitFor, within } from "@testing-library/react";
import fs from "node:fs";
import path from "node:path";
import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("react-i18next", () => ({
  initReactI18next: { type: "3rdParty", init: () => {} },
  useTranslation: () => ({ t: (key: string) => key }),
}));

const invokeMock = vi.fn((..._args: unknown[]) => Promise.resolve(null));
vi.mock("@/lib/invoke", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

// 用真实 store 模块替换 barrel（避免 barrel 拉入无关重依赖）
vi.mock("@/stores", async () => {
  const stock = await import("@/stores/feature/stockAnalysisStore");
  const settings = await import("@/stores/feature/settingsStore");
  return {
    useStockAnalysisStore: stock.useStockAnalysisStore,
    useSettingsStore: settings.useSettingsStore,
  };
});

import { useSettingsStore } from "@/stores/feature/settingsStore";
import { useStockAnalysisStore } from "@/stores/feature/stockAnalysisStore";
import { ReportMarkdown } from "../ReportMarkdown";
import { ValueAssessmentPanel } from "../ValueAssessmentPanel";

// DB 实测样本（固化为 fixture，避免依赖 output/ 临时文件）
// 600089：V74 前形态（margin_of_safety 为字符串）
const REPORT_600089 = fs.readFileSync(
  path.resolve(__dirname, "fixtures/value-investor-600089.txt"),
  "utf8",
);
// 300620：V74 后形态（verdict 字段是 number/null，margin_of_safety=-100、intrinsic_value_range=null）
const REPORT_300620 = fs.readFileSync(
  path.resolve(__dirname, "fixtures/value-investor-300620.txt"),
  "utf8",
);

function seedStore(report: string, stockCode = "300620") {
  useStockAnalysisStore.setState({
    valueAssessments: { "value-investor": report },
    ruleCheckResults: {},
    dataQualitySummary: "",
    rawData: {},
    stockCode,
    // 2026-09-21: 显式重置估值前提标注 —— zustand `setState` 是**浅合并**，
    //   不传该键会让上一个用例设的标注残留到下一个用例（跨用例污染，
    //   症状是「莫名多出一条 Alert」或「本该出现的没出现」）。
    valuationApplicability: null,
  });
  const settingsState = useSettingsStore.getState() as { settings?: { themeMode?: string } };
  if (!settingsState.settings) {
    (useSettingsStore.setState as (s: unknown) => void)({ settings: { themeMode: "dark" } });
  }
}

beforeAll(() => {
  // jsdom 无 matchMedia，组件 isDark 计算需要
  if (!window.matchMedia) {
    window.matchMedia = ((query: string) => ({
      matches: false,
      media: query,
      onchange: null,
      addListener: () => {},
      removeListener: () => {},
      addEventListener: () => {},
      removeEventListener: () => {},
      dispatchEvent: () => false,
    })) as unknown as typeof window.matchMedia;
  }
});

beforeEach(() => {
  invokeMock.mockClear();
});

describe("ValueAssessmentPanel 真实数据渲染复现（页面错误回归）", () => {
  it("600089 完整估值报告（V74 前形态）→ 渲染不抛错", () => {
    seedStore(REPORT_600089, "600089");
    expect(() => render(<ValueAssessmentPanel />)).not.toThrow();
  });

  it("300620 V74 后形态（margin_of_safety 为 number）→ 渲染不抛错", () => {
    seedStore(REPORT_300620);
    // 修复前此处抛 TypeError: initialMarkdown.startsWith is not a function
    // → 整页落入 PageErrorBoundary 的「页面错误」兜底
    expect(() => render(<ValueAssessmentPanel />)).not.toThrow();
  });

  it("store.stockCode 有值时 → 触发 compute_valuation_band（估值带不再是死路）", async () => {
    seedStore(REPORT_300620, "300620");
    render(<ValueAssessmentPanel />);
    await waitFor(() => {
      const called = invokeMock.mock.calls.some(
        (c) => c[0] === "compute_valuation_band" && (c[1] as { stockCode?: string })?.stockCode === "300620",
      );
      expect(called).toBe(true);
    });
  });

  it("stockCode 为空 → 不发请求、不渲染估值带", () => {
    seedStore(REPORT_300620, "");
    render(<ValueAssessmentPanel />);
    expect(invokeMock.mock.calls.some((c) => c[0] === "compute_valuation_band")).toBe(false);
  });
});

describe("ReportMarkdown content 类型收敛契约", () => {
  // markstream 对 content 调 startsWith；非 string 一律在入口收敛，避免打崩整页
  it.each([
    ["number", -100],
    ["null", null],
    ["undefined", undefined],
    ["boolean", false],
    ["object", { verdict: "规避", score: 75 }],
  ])("content=%s → 不抛错", (_label, value) => {
    expect(() => render(<ReportMarkdown content={value as unknown as string} isDark />)).not.toThrow();
  });
});

/**
 * 2026-09-21：估值前提标注（新增消费端）。
 *
 * 背景：`portfolio-mgr` 一直在产 `valuationApplicability`，但前端**零消费** ⇒
 * 用户在面板看到「内在价值 80.63–156.77 元」，却看不到「该区间锚定于
 * 近 5 年正净利均值 ×0.90 的历史代理」。本组锁住两个方向：
 * 命中时必须显示（且在数字之前），没有该字段时**不得**凭空显示。
 */
describe("估值前提标注", () => {
  // 取自 300308 实际产物形状
  const APPLICABILITY = {
    dcfApplicable: false,
    dcfLegUsed: false,
    grahamLegUsed: true,
    reason: "净利为正但当期真实自由现金流与盈利量级脱钩（FCF/净利 = 0.14 < 0.3）",
    anchorIsFallback: true,
    grahamGrowthClamped: true,
    growthBandAllNegative: false,
  };

  it("命中不适用 / 封顶 ⇒ 渲染标注，且**排在估值卡片之前**", () => {
    seedStore(REPORT_300620, "300620");
    useStockAnalysisStore.setState({ valuationApplicability: APPLICABILITY });
    render(<ValueAssessmentPanel />);

    expect(screen.getByText("stockAnalysis.valuationApplicability.title")).toBeTruthy();
    // I1 闸口后该文案出现在两处：前提标注列表 + 被屏蔽的估值结论区 ⇒ 用 getAll
    expect(screen.getAllByText("stockAnalysis.valuationApplicability.dcfNotApplicable").length)
      .toBeGreaterThanOrEqual(1);
    expect(screen.getByText("stockAnalysis.valuationApplicability.grahamGrowthClamped")).toBeTruthy();
    expect(screen.getByText(/0\.14/)).toBeTruthy();

    // 顺序断言：用户必须先看到前提、再看到数字，否则标注形同不存在
    const html = document.body.innerHTML;
    const noticeIdx = html.indexOf("stockAnalysis.valuationApplicability.title");
    const cardIdx = html.indexOf("stockAnalysis.valueAssessment.title");
    expect(noticeIdx).toBeGreaterThan(-1);
    expect(cardIdx).toBeGreaterThan(-1);
    expect(noticeIdx).toBeLessThan(cardIdx);
  });

  it("dcfApplicable=false 时不再重复报「锚定是代理」（两条同因会互相冲淡）", () => {
    seedStore(REPORT_300620, "300620");
    useStockAnalysisStore.setState({ valuationApplicability: APPLICABILITY });
    render(<ValueAssessmentPanel />);
    expect(screen.queryByText("stockAnalysis.valuationApplicability.anchorIsFallback")).toBeNull();
  });

  it("DCF 腿仍在参与但锚定是历史代理 ⇒ 必须报锚定口径", () => {
    seedStore(REPORT_300620, "300620");
    useStockAnalysisStore.setState({
      valuationApplicability: {
        ...APPLICABILITY,
        dcfApplicable: true,
        dcfLegUsed: true,
        grahamGrowthClamped: false,
      },
    });
    render(<ValueAssessmentPanel />);
    // J1 闸口后该文案可出现在前提标注 + 结论区两处 ⇒ getAll
    expect(
      screen.getAllByText("stockAnalysis.valuationApplicability.anchorIsFallback").length,
    ).toBeGreaterThanOrEqual(1);
  });

  it("两条腿都不可用 ⇒ 报「估值维度整体退出」", () => {
    seedStore(REPORT_300620, "300620");
    useStockAnalysisStore.setState({
      valuationApplicability: { ...APPLICABILITY, dcfLegUsed: false, grahamLegUsed: false },
    });
    render(<ValueAssessmentPanel />);
    expect(screen.getByText("stockAnalysis.valuationApplicability.allLegsExcluded")).toBeTruthy();
  });

  it("**反向对照**：valuationApplicability=null ⇒ 一条标注都不渲染（不凭空报警）", () => {
    seedStore(REPORT_300620, "300620");
    render(<ValueAssessmentPanel />);
    expect(screen.queryByText("stockAnalysis.valuationApplicability.title")).toBeNull();
    expect(screen.queryByText("stockAnalysis.valuationApplicability.allLegsExcluded")).toBeNull();
  });
});

/**
 * I1 确定性闸口（2026-09-28，601399 国机重装实证，运行 `de6b0594`）。
 *
 * 病灶：`applicable=false` 时 prompt 硬规则要求 `intrinsic_value_range` 填 null，
 * 但 LLM 实测填了「0.69-0.94元（…前提不成立，仅供参考）」的带免责声明区间，
 * 面板原样渲染成「估值结论」——把引擎已判死的结果当结论展示。
 * 闸口在数据层（结构化布尔 dcfApplicable），不依赖 LLM 守规矩、不解析文案。
 */
describe("I1：DCF 不适用时估值区间不进结论区", () => {
  // 601399 真实违规形态：applicable=false + 字段仍带区间与免责声明
  const REPORT_601399 = fs.readFileSync(
    path.resolve(__dirname, "fixtures/value-investor-601399.txt"),
    "utf8",
  );
  const NOT_APPLICABLE = {
    dcfApplicable: false,
    dcfLegUsed: false,
    grahamLegUsed: true,
    reason: "当期净利 4.83 亿为正但自由现金流 -16.78 亿 ≤ 0（符号相反）：FCF 折现不反映股东可分配",
    anchorIsFallback: true,
    grahamGrowthClamped: false,
    growthBandAllNegative: false,
  };

  it("applicable=false ⇒ 违规区间串（0.69-0.94）与安全边际不得出现在估值结论区", () => {
    seedStore(REPORT_601399, "601399");
    useStockAnalysisStore.setState({ valuationApplicability: NOT_APPLICABLE });
    render(<ValueAssessmentPanel />);
    expect(screen.queryByText(/0\.69/)).toBeNull();
    expect(screen.queryByText(/0\.94/)).toBeNull();
    // 安全边际独立行必须消失；buffett_verdict 正文里 LLM 引用的 -77.4% 属裁决叙述，不在屏蔽范围
    expect(screen.queryByText("-77.4%")).toBeNull();
    // 闸口生效的可见证据：结论区出现「不适用」文案（前提标注 + 结论区至少各一）
    expect(screen.getAllByText("stockAnalysis.valuationApplicability.dcfNotApplicable").length)
      .toBeGreaterThanOrEqual(2);
  });

  it("**反向锁**：dcfApplicable=true ⇒ 区间照常展示（闸口不是无条件屏蔽）", () => {
    seedStore(REPORT_601399, "601399");
    useStockAnalysisStore.setState({
      valuationApplicability: { ...NOT_APPLICABLE, dcfApplicable: true, anchorIsFallback: false },
    });
    render(<ValueAssessmentPanel />);
    expect(screen.getAllByText(/0\.69/).length).toBeGreaterThanOrEqual(1);
  });

  it("J1：dcfApplicable=true 但锚定是历史代理 ⇒ 区间同样不进结论区（只报口径）", () => {
    seedStore(REPORT_601399, "601399");
    useStockAnalysisStore.setState({
      valuationApplicability: { ...NOT_APPLICABLE, dcfApplicable: true, dcfLegUsed: true },
    });
    render(<ValueAssessmentPanel />);
    expect(screen.queryByText(/0\.69/)).toBeNull();
    expect(screen.queryByText(/0\.94/)).toBeNull();
    expect(
      screen.getAllByText("stockAnalysis.valuationApplicability.anchorIsFallback").length,
    ).toBeGreaterThanOrEqual(1);
  });
});

/**
 * V92（2026-09-28，301269 华大九天实证：现价 87.56 元，LLM 给出「2.16–4.42 元 / -97.5%」）。
 *
 * 病灶：面板只渲染 LLM 口径的 `intrinsic_value_range` 并把它标成「估值结论」，
 * 本地算法（反向 DCF + 相对估值）的结论完全不可见 ⇒ 用户以为那就是结论。
 * 处置：`value-verify` 无条件注入顶层 `valuation_conclusion`，面板置顶展示档位与
 * 一句话结论；区间区块标题按归属标为「算法 DCF 估值区间」（V93 修正：该区间本就是
 * `value-verify` 覆写后的算法输出，标成「LLM 估值区间」是归属错误）。
 */
const REPORT_WITH_CONCLUSION = JSON.stringify({
  buffett_verdict: "裁决正文",
  intrinsic_value_range: "2.16-4.42元",
  margin_of_safety: "-97.5%",
  valuation_conclusion: {
    action: "高估",
    headline: "反向 DCF 显示现价隐含 FCF 年复合 130% ⇒ 判为「高估」。",
    primaryMethod: "reverse_dcf",
    relativeVerdict: "rich",
    relativePrimary: "PS",
    reverseFeasibility: "Impossible",
  },
});

describe("V92：算法估值结论置顶", () => {
  it("有 valuation_conclusion ⇒ 结论置顶且区块标题标为算法 DCF 区间", () => {
    seedStore(REPORT_WITH_CONCLUSION, "301269");
    render(<ValueAssessmentPanel />);

    expect(screen.getByText("stockAnalysis.valueAssessment.algorithmConclusion")).toBeTruthy();
    expect(screen.getByText(/130%/)).toBeTruthy();
    expect(screen.getByText("高估")).toBeTruthy();
    // 信息不丢：区间仍渲染，但标题必须标为算法 DCF 输出，不能标成「LLM 估值区间」
    expect(screen.getByText("stockAnalysis.valueAssessment.algorithmRangeBand")).toBeTruthy();
    expect(screen.queryByText("stockAnalysis.valueAssessment.valuationConclusion")).toBeNull();

    // 顺序断言：算法结论必须先于价值评估卡片出现
    const html = document.body.innerHTML;
    const algoIdx = html.indexOf("stockAnalysis.valueAssessment.algorithmConclusion");
    const cardIdx = html.indexOf("stockAnalysis.valueAssessment.title");
    expect(algoIdx).toBeGreaterThan(-1);
    expect(cardIdx).toBeGreaterThan(-1);
    expect(algoIdx).toBeLessThan(cardIdx);
  });

  it("**反向锁**：无 valuation_conclusion ⇒ 不显示结论区（不伪造算法结论）", () => {
    seedStore(REPORT_300620, "300620");
    render(<ValueAssessmentPanel />);
    expect(screen.queryByText("stockAnalysis.valueAssessment.algorithmConclusion")).toBeNull();
    expect(screen.queryByText("stockAnalysis.valueAssessment.algorithmRangeBand")).toBeNull();
  });
});

/**
 * K3（2026-10-02，600276 恒瑞医药实证，样本 `21cdd00e`）。
 *
 * 病灶：三档预测期增速 -2.71% / -1.94% / -1.16% **全部为负** —— 连「乐观档」都假设营收
 * 逐年萎缩，而面板标题写「算法 DCF 估值区间（保守档—乐观档）」⇒ 标签与模型实际假设不符。
 * 该形态下 `dcfApplicable=true` 且 `anchorIsFallback=false`（锚是真实年报 FCF 82.73 亿），
 * 所以 I1/J1 两道闸口**全部放行** —— 这是前两轮修复覆盖不到的第四族出口。
 * 判据走结构化布尔（`growthBandAllNegative`），不解析 LLM 文案。
 */
describe("K3：三档增速全负时区间必须改口", () => {
  const DECLINE = {
    dcfApplicable: true,
    dcfLegUsed: true,
    grahamLegUsed: true,
    reason: "",
    anchorIsFallback: false,
    grahamGrowthClamped: false,
    growthBandAllNegative: true,
  };

  it("命中 ⇒ 标题换成衰退带 + 前提标注出现，且旧标题不残留", () => {
    seedStore(REPORT_WITH_CONCLUSION, "301269");
    useStockAnalysisStore.setState({ valuationApplicability: DECLINE });
    render(<ValueAssessmentPanel />);
    expect(
      screen.getByText("stockAnalysis.valueAssessment.algorithmRangeBandDecline"),
    ).toBeTruthy();
    expect(
      screen.getByText("stockAnalysis.valuationApplicability.growthBandAllNegative"),
    ).toBeTruthy();
    // 同时出现两个标题 = 给了两个口径，属新增歧义 ⇒ 必须互斥
    expect(screen.queryByText("stockAnalysis.valueAssessment.algorithmRangeBand")).toBeNull();
    // 区间数值本身**仍然展示**：K3 只纠措辞，不屏蔽（锚是真的，数值可用）
    expect(screen.getByText(/2\.16-4\.42/)).toBeTruthy();
  });

  it("**反向锁**：growthBandAllNegative=false ⇒ 两条新文案都不得出现", () => {
    seedStore(REPORT_WITH_CONCLUSION, "301269");
    useStockAnalysisStore.setState({
      valuationApplicability: { ...DECLINE, growthBandAllNegative: false },
    });
    render(<ValueAssessmentPanel />);
    expect(
      screen.queryByText("stockAnalysis.valueAssessment.algorithmRangeBandDecline"),
    ).toBeNull();
    expect(
      screen.queryByText("stockAnalysis.valuationApplicability.growthBandAllNegative"),
    ).toBeNull();
    expect(screen.getByText("stockAnalysis.valueAssessment.algorithmRangeBand")).toBeTruthy();
  });

  it("DCF 腿已被剔除 ⇒ 不重复报（区间本就不展示，报了是噪声）", () => {
    seedStore(REPORT_WITH_CONCLUSION, "301269");
    useStockAnalysisStore.setState({
      valuationApplicability: { ...DECLINE, dcfApplicable: false },
    });
    render(<ValueAssessmentPanel />);
    expect(
      screen.queryByText("stockAnalysis.valuationApplicability.growthBandAllNegative"),
    ).toBeNull();
  });
});

// 四周期回归：value 链按档实例化（`value-investor--mid|long`），面板此前只读裸键
// `valueAssessments["value-investor"]` ⇒ 带档产物取不到、「巴菲特估值」主卡恒不渲染
// （DB 实证 a3eba895：该链只产 value.assessment--mid|long）。锁死归一收集行为。
describe("ValueAssessmentPanel 逐档 value 槽位（四周期回归）", () => {
  const TIERED = { "value-investor--mid": REPORT_600089, "value-investor--long": REPORT_300620 };

  function seedTiered(report: Record<string, string>) {
    useStockAnalysisStore.setState({
      valueAssessments: report,
      ruleCheckResults: {},
      dataQualitySummary: "",
      rawData: {},
      stockCode: "600089",
      valuationApplicability: null,
    });
    const settingsState = useSettingsStore.getState() as { settings?: { themeMode?: string } };
    if (!settingsState.settings) {
      (useSettingsStore.setState as (s: unknown) => void)({ settings: { themeMode: "dark" } });
    }
  }

  it("只写带档键 ⇒ 主卡仍渲染（裸键回退不再必要）", () => {
    seedTiered({ "value-investor--mid": REPORT_600089 });
    render(<ValueAssessmentPanel />);
    // 主卡标题（buffettLabel 与 title 两个 i18n 键都在）
    expect(screen.getByText("stockAnalysis.valueAssessment.buffettLabel")).toBeTruthy();
    expect(screen.getByText("stockAnalysis.valueAssessment.title")).toBeTruthy();
  });

  it("多档 ⇒ 出现档位切换条（中线 / 长线各一个）", () => {
    seedTiered(TIERED);
    render(<ValueAssessmentPanel />);
    // 切换条内每档一个 chip；用 testid 限定作用域 —— 当前档的标签同时出现在卡片标题的 Tag 里，
    // 全局 getByText 会命中两个（2026-10-09 首次实测即踩）
    const bar = screen.getByTestId("value-tier-switch");
    expect(within(bar).getByText("stockAnalysis.timeHorizonMid")).toBeTruthy();
    expect(within(bar).getByText("stockAnalysis.timeHorizonLong")).toBeTruthy();
    expect(within(bar).getAllByRole("button")).toHaveLength(2);
  });

  it("裸键形态（≤v132 历史快照）⇒ 单卡、无切换条", () => {
    seedTiered({ "value-investor": REPORT_600089 });
    render(<ValueAssessmentPanel />);
    expect(screen.getByText("stockAnalysis.valueAssessment.buffettLabel")).toBeTruthy();
    expect(screen.queryByText("stockAnalysis.timeHorizonMid")).toBeNull();
  });
});
