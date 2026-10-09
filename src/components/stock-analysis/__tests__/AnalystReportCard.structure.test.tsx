// 分析师卡的**正文缺失**回归门禁（2026-10-09）。
//
// 背景（用户报「技术分析师超短线卡片只有一行『分析完成，但未返回结构化内容』」、
//   「催化剂与叙事分析师中线卡片只有结论头部 + chips、没有正文」）：
//   `agency_experts/stock-analysis/*.md` 明确要求「必须有正文，只有 VERDICT 标签视为无效」，
//   但实测 LLM 会违规输出「只有标签」或「形状未知的 JSON」。此时既有实现把内容**整段丢掉**：
//     ① `bull_points` / `bear_points`（标签里的多空论据）从未被读取 ⇒ 只剩头部 + chips；
//     ② 形状未知的 JSON 只显示「未返回结构化内容」⇒ 用户看不到任何当次分析内容。
//   本测试锁住两条降级路径，防止回归为「有内容却显示为空」。
//
// i18n 与 ReportMarkdown 刻意 mock：断言的是「组件把哪些字段渲染出来」，不是译文/排版。
import { render } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { AnalystReportCard } from "../AnalystReportCard";

vi.mock("react-i18next", () => ({
  initReactI18next: { type: "3rdParty", init: () => {} },
  useTranslation: () => ({ t: (key: string) => key }),
}));

vi.mock("markstream-react", () => ({ setCustomComponents: () => {} }));

vi.mock("@/stores", () => ({
  useSettingsStore: (sel: (s: { settings: { themeMode: string } }) => unknown) =>
    sel({ settings: { themeMode: "dark" } }),
}));

vi.mock("../ReportMarkdown", () => ({
  ReportMarkdown: ({ content }: { content: string }) => <div>{content}</div>,
}));

vi.mock("../AnalystDataQualityModal", () => ({ AnalystDataQualityModal: () => null }));

function textOf(report: string): string {
  const { container } = render(
    <AnalystReportCard expertId="market-analyst" tierLabel="超短线" report={report} />,
  );
  return container.textContent ?? "";
}

describe("AnalystReportCard — 正文缺失时的降级渲染（2026-10-09）", () => {
  it("只有 VERDICT 标签、无正文 ⇒ 必须渲染标签里的 bull_points / bear_points", () => {
    const report = `<!-- VERDICT: {"verdict":"中性","bull_score":48,"bear_score":52,"confidence":55,`
      + `"bull_points":["均线金叉"],"bear_points":["上方套牢盘密集"]} -->`;
    const text = textOf(report);
    expect(text).toContain("均线金叉");
    expect(text).toContain("上方套牢盘密集");
    // 有正文可渲染 ⇒ 不得再落「未返回结构化内容」
    expect(text).not.toContain("stockAnalysis.analystReport.completedNoStructure");
  });

  it("形状未知的 JSON ⇒ 平铺其余可读字段，而不是「未返回结构化内容」", () => {
    const text = textOf('{"technical_summary":"均线纠缠，量能萎缩","support":28.5}');
    expect(text).toContain("technical_summary");
    expect(text).toContain("均线纠缠，量能萎缩");
    expect(text).toContain("support");
    expect(text).not.toContain("stockAnalysis.analystReport.completedNoStructure");
  });

  it("真正的空对象 ⇒ 仍落「未返回结构化内容」（兜底句只属于无内容）", () => {
    const text = textOf("{}");
    expect(text).toContain("stockAnalysis.analystReport.completedNoStructure");
  });
});
