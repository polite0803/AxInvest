// 估值评估 tab「页面错误」复现 + 回归测试：
// 用 DB 实际 value-investor 输出渲染 ValueAssessmentPanel，并锁定 ReportMarkdown 的 content 收敛契约
import { render, screen, waitFor } from "@testing-library/react";
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
  };

  it("命中不适用 / 封顶 ⇒ 渲染标注，且**排在估值卡片之前**", () => {
    seedStore(REPORT_300620, "300620");
    useStockAnalysisStore.setState({ valuationApplicability: APPLICABILITY });
    render(<ValueAssessmentPanel />);

    expect(screen.getByText("stockAnalysis.valuationApplicability.title")).toBeTruthy();
    expect(screen.getByText("stockAnalysis.valuationApplicability.dcfNotApplicable")).toBeTruthy();
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
    expect(screen.getByText("stockAnalysis.valuationApplicability.anchorIsFallback")).toBeTruthy();
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
