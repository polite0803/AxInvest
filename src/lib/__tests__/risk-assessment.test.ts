/**
 * 风险产物解析（三视角 / 规则档）。
 *
 * 夹具按现网形态：三视角是 AgentResult 包装 → content 是内层 JSON
 * `{report, verdict:{stance,confidence,position_pct}}`；规则档是扁平的
 * `{category, matched_rules, axisScope, metrics}`。
 *
 * 这里锁的是「只取值、不打分」：verdict 三个字段原样返回，confidence 不转成风险分；
 * 规则档认不出结构化字段时返回 null，交给调用方按普通产物保留（不静默丢）。
 */
import { isRiskAggregateKey, parseRiskAssessment, parseRiskLevel } from "@/lib/risk-assessment";
import { describe, expect, it } from "vitest";

function agentResult(content: string): string {
  return JSON.stringify({
    role: "stock-investment-lead",
    model: "agnes-3.0-flash",
    usage: { input_tokens: 1, output_tokens: 2 },
    content,
  });
}

const RISK_AGG = agentResult(JSON.stringify({
  report: "# 激进风险评估\n\n正文 A",
  verdict: { stance: "aggressive", confidence: 45, position_pct: 20 },
  __verdict_only: false,
}));

const RISK_LEVEL = JSON.stringify({
  axisScope: {
    drawdownDepth: "absent",
    fundamentals: "latestReport",
    sharpe: "global60",
    volatility: "global60",
  },
  category: "极高风险",
  degraded: false,
  matched_rules: ["EXTREME-3: 年化波动率>60% 且 夏普比率<-1.5"],
  metrics: {
    debt_ratio_pct: 64.1,
    drawdown_depth: null,
    sharpe: -2.349,
    volatility_pct: 70.1,
    window_days: null,
  },
  warnings: [],
});

describe("parseRiskAssessment — 三视角 verdict 与正文", () => {
  it("解 AgentResult 包装后取 verdict 三字段与 report 正文", () => {
    const view = parseRiskAssessment(RISK_AGG);
    expect(view.verdict).toEqual({ stance: "aggressive", positionPct: 20, confidence: 45 });
    expect(view.report).toBe("# 激进风险评估\n\n正文 A");
  });

  it("顶层 verdict（无嵌套）同样认", () => {
    const view = parseRiskAssessment(JSON.stringify({
      report: "正文 B",
      stance: "conservative",
      confidence: 55,
      position_pct: 0,
    }));
    expect(view.verdict).toEqual({ stance: "conservative", positionPct: 0, confidence: 55 });
    expect(view.report).toBe("正文 B");
  });

  it("纯 Markdown 原样作为正文，verdict 全为 null（不伪造 0）", () => {
    const view = parseRiskAssessment("## 保守评估\n\n没有结构化结论。");
    expect(view.verdict).toEqual({ stance: null, positionPct: null, confidence: null });
    expect(view.report).toBe("## 保守评估\n\n没有结构化结论。");
  });

  it("解析成功但没有 report ⇒ 正文为空串，不把 JSON 原文当正文", () => {
    const view = parseRiskAssessment(JSON.stringify({
      verdict: { stance: "neutral", confidence: 45, position_pct: 20 },
    }));
    expect(view.report).toBe("");
    expect(view.verdict.stance).toBe("neutral");
  });
});

describe("parseRiskLevel — 规则风险档", () => {
  it("category / matched_rules / axisScope / metrics 全量取出", () => {
    const level = parseRiskLevel(RISK_LEVEL);
    expect(level).not.toBeNull();
    expect(level!.category).toBe("极高风险");
    expect(level!.matchedRules).toEqual(["EXTREME-3: 年化波动率>60% 且 夏普比率<-1.5"]);
    expect(level!.axisScope.drawdownDepth).toBe("absent");
    expect(level!.metrics.volatility_pct).toBe(70.1);
    expect(level!.metrics.drawdown_depth).toBeNull();
    expect(level!.degraded).toBe(false);
  });

  it("只有 metrics 等任一结构化字段也认（不要求 category）", () => {
    const level = parseRiskLevel(JSON.stringify({ metrics: { sharpe: -1.2 } }));
    expect(level).not.toBeNull();
    expect(level!.category).toBeNull();
    expect(level!.metrics.sharpe).toBe(-1.2);
  });

  it("普通报告 JSON 不是规则档 ⇒ null（调用方按普通产物保留）", () => {
    expect(parseRiskLevel(JSON.stringify({ report: "# 正文", verdict: { stance: "neutral" } }))).toBeNull();
    expect(parseRiskLevel("纯文本")).toBeNull();
  });
});

describe("isRiskAggregateKey", () => {
  it("聚合容器（不是第四个视角）", () => {
    expect(isRiskAggregateKey("risk-aggregated")).toBe(true);
    expect(isRiskAggregateKey("agg-risk")).toBe(true);
    expect(isRiskAggregateKey("risk-agg")).toBe(false);
    expect(isRiskAggregateKey("risk-con")).toBe(false);
    expect(isRiskAggregateKey("risk-neu")).toBe(false);
  });
});
