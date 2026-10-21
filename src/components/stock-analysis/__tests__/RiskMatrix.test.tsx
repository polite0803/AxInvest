/**
 * 风险评估主面板（三视角 / 规则档 / 收敛 / 研究经理）。
 *
 * 修复前：三视角与 risk-level / risk-convergence / research-mgr 平铺，靠
 * confidence 算「风险分」并画成一条雷达；risk-level 的纯 JSON 会原样上屏。
 * 这里锁新的呈现契约：
 *   ① 三视角固定顺序，立场/建议仓位/置信度是 verdict 原值；
 *   ② 缺票明说，不拿别的视角顶替；
 *   ③ 规则档显示 category / 命中规则 / 口径缺席，不出现原始 JSON 字段名；
 *   ④ 收敛结论置顶；研究经理默认折叠；
 *   ⑤ 聚合容器不算第四个视角；雷达图已移除。
 */
import i18n from "@/i18n";
import { useStockAnalysisStore } from "@/stores";
import { fireEvent, render, screen } from "@testing-library/react";
import { I18nextProvider } from "react-i18next";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { RiskMatrix } from "../RiskMatrix";

const invokeMock = vi.fn();
vi.mock("@/lib/invoke", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
  listen: vi.fn().mockResolvedValue(() => {}),
  isTauri: () => false,
}));

function assessment(
  report: string,
  stance: "aggressive" | "conservative" | "neutral",
  confidence: number,
  positionPct: number,
): string {
  return JSON.stringify({ report, verdict: { stance, confidence, position_pct: positionPct } });
}

const RISK_LEVEL = JSON.stringify({
  axisScope: { drawdownDepth: "absent", fundamentals: "latestReport", sharpe: "global60", volatility: "global60" },
  category: "极高风险",
  degraded: false,
  matched_rules: ["EXTREME-3: 年化波动率>60% 且 夏普比率<-1.5"],
  metrics: { volatility_pct: 70.1, sharpe: -2.349, max_drawdown_pct: 37.9, debt_ratio_pct: 64.1 },
  warnings: [],
});

function fullAssessments(): Record<string, string> {
  return {
    "risk-agg": assessment("激进正文：对称赔率下建议左侧布局。", "aggressive", 45, 20),
    "risk-con": assessment("保守正文：等待右侧信号再考虑。", "conservative", 55, 0),
    "risk-neu": assessment("中性正文：维持观察。", "neutral", 45, 20),
    "risk-level": RISK_LEVEL,
    "risk-convergence": JSON.stringify({ report: "## 收敛结论正文\n\n三方仓位分歧较大。" }),
    "research-mgr": JSON.stringify({ report: "## 研究经理正文\n\n综合意见。" }),
    "risk-aggregated": JSON.stringify([{ content: "聚合容器不是第四个视角" }]),
  };
}

function renderPanel() {
  return render(
    <I18nextProvider i18n={i18n}>
      <RiskMatrix />
    </I18nextProvider>,
  );
}

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockResolvedValue(null);
  useStockAnalysisStore.getState().reset();
});

describe("RiskMatrix — 三视角与规则档的呈现", () => {
  it("三视角固定顺序，展示 verdict 的立场/仓位/置信度（不显示风险分）", () => {
    useStockAnalysisStore.setState({ riskAssessments: fullAssessments() });
    const { container } = renderPanel();

    const text = container.textContent ?? "";
    // 仓位与置信度是 verdict 原值：激进 20%/45、保守 0%/55、中性 20%/45
    expect((text.match(/建议仓位：20%/g) ?? []).length).toBe(2);
    expect(text).toContain("建议仓位：0%");
    expect((text.match(/置信度：45/g) ?? []).length).toBe(2);
    expect(text).toContain("置信度：55");
    // 旧「风险分: NN」不再出现（注意别误伤 HorizonScopeNotice 的「风险分类：」）
    expect(text).not.toContain("风险分:");

    // 顺序：激进 → 保守 → 中性
    const act = text.indexOf("激进");
    const con = text.indexOf("保守");
    const neu = text.indexOf("中性");
    expect(act).toBeGreaterThanOrEqual(0);
    expect(act).toBeLessThan(con);
    expect(con).toBeLessThan(neu);
  });

  it("缺一票时明确说「本视角未产出」，不拿别的视角顶替", () => {
    const data = fullAssessments();
    delete data["risk-neu"];
    useStockAnalysisStore.setState({ riskAssessments: data });
    renderPanel();

    expect(screen.getByText(i18n.t("stockAnalysis.riskMatrix.noVote"))).toBeTruthy();
    // 缺失视角不继承超短/保守的正文
    expect(screen.queryByText(/中性正文/)).toBeNull();
  });

  it("规则档显示类别/命中规则/口径缺席，不显示原始 JSON 字段名", () => {
    useStockAnalysisStore.setState({ riskAssessments: fullAssessments() });
    const { container } = renderPanel();

    const text = container.textContent ?? "";
    expect(screen.getByText("极高风险")).toBeTruthy();
    expect(text).toContain("EXTREME-3");
    expect(text).toContain("回撤深度=缺席");
    expect(text).toContain("年化波动率");
    expect(text).toContain("70.1%");
    // 旧实现会把 488B 的规则档 JSON 整段渲染出来
    expect(text).not.toContain("matched_rules");
    expect(text).not.toContain("axisScope");
    expect(text).not.toContain('"category"');
  });

  it("收敛结论置顶，研究经理默认折叠", () => {
    useStockAnalysisStore.setState({ riskAssessments: fullAssessments() });
    renderPanel();

    expect(screen.getByText("三视角收敛结论")).toBeTruthy();
    expect(screen.getByText(/收敛结论正文/)).toBeTruthy();
    expect(screen.queryByText(/研究经理正文/)).toBeNull();

    fireEvent.click(screen.getByText("研究经理综合意见"));
    expect(screen.getByText(/研究经理正文/)).toBeTruthy();
  });

  it("聚合容器不算第四个视角，雷达图已移除", () => {
    useStockAnalysisStore.setState({ riskAssessments: fullAssessments() });
    const { container } = renderPanel();

    expect(screen.queryByText("其他风险产物")).toBeNull();
    expect(container.querySelector("canvas")).toBeNull();
  });
});
