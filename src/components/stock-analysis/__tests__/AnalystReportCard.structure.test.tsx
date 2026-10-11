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

// 2026-10-10 回归（用户报 002475 历史记录「分析师卡片没有看多/看空/置信度结论」、
// 「卡片正文裸 camelCase 键值平铺」）。落库实证 24 个分析师节点里 4 个产出**裸因子 JSON**
// `{direction,confidence,因子…}`（无 report、无 verdict 字段），value-investor 则是
// `{report,verdict:{},valuation_conclusion:{action}}` 外壳 —— 旧实现两类都不渲染结论徽章。
describe("AnalystReportCard — 裸因子 JSON / value-investor 外壳的结论与因子渲染（2026-10-10）", () => {
  it("裸因子对象：direction 提升为结论徽章，置信度照常显示", () => {
    const report = JSON.stringify({
      direction: "偏多",
      confidence: 72,
      earningsQuality: "qualityRealizationBeyondHorizon：营收高增但利润承压",
      valuationBand: "低估（deep_value）",
      exitRule: "k_sigma_band",
    });
    const text = textOf(report);
    // 「偏多」经真实 classifyDirectionText 判为 bull ⇒ 渲染看多徽章（t mock 回显 key）
    expect(text).toContain("stockAnalysis.analystVerdict.bullish");
    // 置信度不再因缺 verdict 字段被整段隐藏
    expect(text).toContain("stockAnalysis.confidence");
    expect(text).toContain("72%");
    // 不落「未返回结构化内容」
    expect(text).not.toContain("stockAnalysis.analystReport.completedNoStructure");
  });

  it("裸因子对象：因子按「中文标签 + 值」结构化，剥离 marker 前缀，不再裸平铺 camelCase", () => {
    const report = JSON.stringify({
      direction: "中性",
      confidence: 50,
      earningsQuality: "qualityRealizationBeyondHorizon：营收高增但利润承压",
      qualified: { earningsQuality: "qualityRealizationBeyondHorizon" },
    });
    const text = textOf(report);
    // 因子走 i18n 标签
    expect(text).toContain("stockAnalysis.analystReport.factors.earningsQuality");
    // 值里的 marker camelCase 前缀被剥离，只留中文说明
    expect(text).toContain("营收高增但利润承压");
    expect(text).not.toContain("qualityRealizationBeyondHorizon：");
    // qualified 已被消费，不裸平铺 JSON
    expect(text).not.toContain('"earningsQuality":"qualityRealizationBeyondHorizon"');
  });

  it("notApplicable 因子名数组 ⇒ 走专门的中文化区块，而非混进要点裸显 camelCase", () => {
    const report = JSON.stringify({
      direction: "中性",
      confidence: 40,
      notApplicable: ["momentumSignal", "macroRegime"],
    });
    const text = textOf(report);
    expect(text).toContain("stockAnalysis.analystReport.factorNotApplicable");
    expect(text).toContain("stockAnalysis.analystReport.factors.momentumSignal");
    expect(text).toContain("stockAnalysis.analystReport.factors.macroRegime");
  });

  it("value-investor 外壳：verdict 为空对象时从 valuation_conclusion.action 兜底结论", () => {
    const report = JSON.stringify({
      report: "Let me carefully analyze the moat and valuation of this business in detail.",
      verdict: {},
      valuation_conclusion: { action: "低估", range: "4.44-5.57" },
    });
    const text = textOf(report);
    // 「低估」非多空词也非中性词 ⇒ 徽章如实显示原文，而不是误标「中性」
    expect(text).toContain("低估");
    expect(text).not.toContain("stockAnalysis.analystVerdict.neutral");
  });

  it("标准 {report, verdict:{verdict,bull_score,confidence}} 形态不被归一逻辑破坏", () => {
    const report = JSON.stringify({
      report: "均线多头排列且放量突破前高，趋势确认延续，资金持续流入。",
      verdict: { verdict: "看多", bull_score: 70, bear_score: 20, confidence: 80 },
    });
    const text = textOf(report);
    expect(text).toContain("stockAnalysis.analystVerdict.bullish");
    expect(text).toContain("均线多头排列");
    expect(text).toContain("80%");
  });
});

// 2026-10-10 格式统一回归（用户报「各分析师卡片格式依旧没有统一」，002475 全量 24 节点
// 取证）：① 2 个 VERDICT 标签尾部多吐 `}` 严格解析失败，徽章/分数/论据整段丢失；
// ② notApplicable 混用后端 na_key 文案键（notApplicableMomentum）⇒ 裸显英文；
// ③ qualified 被静默吞掉；④ 多空论据合并成一个混合 ul，丢掉红绿归属。
describe("AnalystReportCard — VERDICT 标签容错与红绿/NA/qualified 统一（2026-10-10）", () => {
  it("标签 JSON 末尾多余 `}` ⇒ 剥离修复，徽章/分数/多空论据全部救回且正文保留", () => {
    // 完整对象后多一个闭合括号（002475 a-hot-money--mid / a-research--long 的真实损坏形态）
    const report = "游资快进快出，板块轮动加速，追高性价比低。\n"
      + '<!-- VERDICT: {"verdict":"偏空","direction":"bear","confidence":55,'
      + '"bull_score":40,"bear_score":60,'
      + '"bull_points":["龙头封板坚决"],"bear_points":["上方套牢盘密集","量能背离"],'
      + '"notApplicable":["notApplicableMomentum"]} } -->';
    const text = textOf(report);
    // 旧实现 catch 后只剩正文；修复后结构化字段全部回归
    expect(text).toContain("stockAnalysis.analystVerdict.bearish");
    expect(text).toContain("龙头封板坚决");
    expect(text).toContain("上方套牢盘密集");
    expect(text).toContain("55%");
    expect(text).toContain("游资快进快出");
    expect(text).not.toContain("stockAnalysis.analystReport.completedNoStructure");
  });

  it("notApplicable 放后端 na_key 文案键 ⇒ 反查因子中文名，不裸显 notApplicableXxx", () => {
    const report = '<!-- VERDICT: {"verdict":"中性","confidence":50,'
      + '"notApplicable":["notApplicableMomentum","notApplicableMacro","sectorRotation"]} -->';
    const text = textOf(report);
    expect(text).toContain("stockAnalysis.analystReport.factorNotApplicable");
    expect(text).toContain("stockAnalysis.analystReport.factors.momentumSignal");
    expect(text).toContain("stockAnalysis.analystReport.factors.macroRegime");
    // 两种形态可混存：因子键直接翻译
    expect(text).toContain("stockAnalysis.analystReport.factors.sectorRotation");
  });

  it("qualified 条件采用 ⇒ 独立区块中文化呈现（此前被 CONSUMED_KEYS 静默吞掉）", () => {
    const report = JSON.stringify({
      direction: "中性",
      confidence: 60,
      qualified: { earningsQuality: "qualityRealizationBeyondHorizon", supplyShock: "absorbed" },
    });
    const text = textOf(report);
    expect(text).toContain("stockAnalysis.analystReport.factorQualified");
    expect(text).toContain("stockAnalysis.analystReport.factors.earningsQuality");
    expect(text).toContain("stockAnalysis.analystReport.factors.supplyShock");
  });

  it("bull_points / bear_points ⇒ 渲染红绿两个分区标题（不再混合成一个列表）", () => {
    const report = '<!-- VERDICT: {"verdict":"看多","bull_score":62,"bear_score":38,"confidence":60,'
      + '"bull_points":["均线金叉"],"bear_points":["套牢盘"]} -->';
    const text = textOf(report);
    expect(text).toContain("stockAnalysis.analystReport.bullArgument");
    expect(text).toContain("stockAnalysis.analystReport.bearArgument");
    expect(text).toContain("均线金叉");
    expect(text).toContain("套牢盘");
  });

  it("标签只给 direction（无 verdict）⇒ direction 兜底为结论徽章", () => {
    const report = '<!-- VERDICT: {"direction":"bear","confidence":58} -->';
    const text = textOf(report);
    expect(text).toContain("stockAnalysis.analystVerdict.bearish");
    expect(text).toContain("58%");
  });

  it("因子机器枚举值（k_sigma_band / not_assessed）⇒ 等宽字体；中文叙述正常字体", () => {
    const report = JSON.stringify({
      direction: "偏多",
      confidence: 70,
      exitRule: "k_sigma_band",
      eventCatalyst: "九月苹果新机发布催化",
    });
    const { container } = render(
      <AnalystReportCard expertId="market-analyst" tierLabel="中线" report={report} />,
    );
    const dds = Array.from(container.querySelectorAll(".sa-factor-list dd"));
    const mono = dds.find((dd) => dd.textContent === "k_sigma_band");
    const cn = dds.find((dd) => dd.textContent?.includes("苹果新机"));
    expect(mono?.className).toContain("font-mono");
    expect(cn?.className ?? "").not.toContain("font-mono");
  });

  it("合法 TAG（无尾随损坏）不被容错逻辑误伤，bull/bear 分数不再以 tag chip 重复", () => {
    const report = "正文内容，足够长不会触发空态判断的正文内容。\n"
      + '<!-- VERDICT: {"verdict":"看多","bull_score":70,"bear_score":20,"confidence":80} -->';
    const { container } = render(
      <AnalystReportCard expertId="market-analyst" tierLabel="中线" report={report} />,
    );
    const text = container.textContent ?? "";
    expect(text).toContain("stockAnalysis.analystVerdict.bullish");
    // 徽章区已用独立样式呈现分数，tags 里不得再出现「看涨:70」chip
    expect(text).not.toContain("stockAnalysis.sentimentBullish:70");
  });
});

// 2026-10-10 用户要求：「所有卡片都不要输出标题，类似『国泰海通（601211）基本面估值
// 分析』的内容都去掉」。卡片头部已有分析师名 + 档位，正文首行 LLM 又写 H1/H2 文档
// 大标题（股票名+代码+分析类型），每张卡重复 ⇒ 剥除；章节标题必须保留。
describe("AnalystReportCard — 文档式大标题剥除（2026-10-10）", () => {
  it("用户原例：H1『国泰海通（601211）基本面估值分析』不渲染，正文保留", () => {
    const report = "# 国泰海通（601211）基本面估值分析\n\n"
      + "均线多头排列且放量突破前高，趋势确认延续，资金持续流入。\n"
      + '<!-- VERDICT: {"verdict":"看多","confidence":80} -->';
    const text = textOf(report);
    expect(text).not.toContain("国泰海通（601211）基本面估值分析");
    expect(text).not.toContain("国泰海通");
    expect(text).toContain("均线多头排列");
  });

  it("无代码但以『报告』结尾的 H2 文档标题同样剥除（如『催化剂与叙事完整度评估报告』）", () => {
    const report = "## 催化剂与叙事完整度评估报告\n\n"
      + "### 一、近期公告梳理与催化剂判断\n近一周公司发布三季报预增公告与股权激励计划，"
      + "机构调研频次明显上升，正文内容足够长不会被空态逻辑误判。";
    const text = textOf(report);
    expect(text).not.toContain("催化剂与叙事完整度评估报告");
    // 紧随的章节标题保留
    expect(text).toContain("一、近期公告梳理");
  });

  it("标题前有一行短引导语 ⇒ 引导语保留，文档标题剥除（a-fundamentals 真实形态）", () => {
    const report = "已收到——工具数据已足够，下面直接出最终报告。\n\n"
      + "## 立讯精密（002475）基本面估值分析\n\n营收高增但利润承压，估值处于历史低位。";
    const text = textOf(report);
    expect(text).toContain("已收到——工具数据已足够");
    expect(text).not.toContain("立讯精密（002475）基本面估值分析");
    expect(text).toContain("营收高增但利润承压");
  });

  it("章节标题 `## 数据概览` 不属于文档标题 ⇒ 保留（policy/research/sector 节点）", () => {
    const report = "## 数据概览\n\n- **机构覆盖与密度**：近12个月获取研报20篇，覆盖密度高且持续。";
    const text = textOf(report);
    expect(text).toContain("数据概览");
    expect(text).toContain("获取研报20篇");
  });

  it("章节标题 `### 1. 趋势状态判定` ⇒ 保留（market-analyst--short 真实形态）", () => {
    const report = "基于 K 线数据与短线契约，技术面分析如下：\n\n"
      + "### 1. 趋势状态判定\n\n均线系统空头排列，短期趋势向下，内容足够长。";
    const text = textOf(report);
    expect(text).toContain("趋势状态判定");
    expect(text).toContain("均线系统空头排列");
  });
});
