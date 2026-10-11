// i18n-exempt: 计票口径单测，断言全部为结构化输入/输出，无 UI 文案。
import { cleanToolCallTags } from "@/lib/agentOutput";
import {
  classifyAnalystReport,
  deriveAnalystConsensus,
  extractBullBearScores,
  tallyAnalystSentiment,
} from "@/lib/stock-analysis-utils";
import { describe, expect, it } from "vitest";

describe("extractBullBearScores", () => {
  it("解析 VERDICT 注释中的 0-100 分制", () => {
    const report = '正文<!-- VERDICT: {"bull_score":70,"bear_score":30} -->';
    expect(extractBullBearScores(report)).toEqual({ bull: 70, bear: 30 });
  });

  it("将 VERDICT 中的 0-1 分制归一化到 0-100（与 0-100 混排时阈值判定才一致）", () => {
    const report = '正文<!-- VERDICT: {"bull_score":0.7,"bear_score":0.3} -->';
    expect(extractBullBearScores(report)).toEqual({ bull: 70, bear: 30 });
  });

  it("VERDICT 只有 stance/verdict 文本、无分数时按方向词给 60 票", () => {
    expect(extractBullBearScores('<!-- VERDICT: {"stance":"增持"} -->'))
      .toEqual({ bull: 60, bear: 0 });
    expect(extractBullBearScores('<!-- VERDICT: {"verdict":"看空"} -->'))
      .toEqual({ bull: 0, bear: 60 });
  });

  it("无 VERDICT 注释时回退匹配正文裸 bull_score/bear_score 字段", () => {
    const report = '分析师写道 "bull_score": 65, "bear_score": 35，谨慎乐观。';
    expect(extractBullBearScores(report)).toEqual({ bull: 65, bear: 35 });
  });

  it("没有任何结构化分数返回 null（调用方走关键词兜底）", () => {
    expect(extractBullBearScores("纯文字报告，建议观望。")).toBeNull();
  });
});

describe("classifyAnalystReport", () => {
  it("结构化分数领先超过 20% 才出单边方向", () => {
    expect(classifyAnalystReport('<!-- VERDICT: {"bull_score":80,"bear_score":20} -->'))
      .toBe("bullish");
    expect(classifyAnalystReport('<!-- VERDICT: {"bull_score":20,"bear_score":80} -->'))
      .toBe("bearish");
  });

  it("结构化分数接近（未达 1.2 倍）判中性，不被微弱差值带偏", () => {
    // 55 vs 60：55 不 > 72，60 不 > 66 ⇒ 中性
    expect(classifyAnalystReport('<!-- VERDICT: {"bull_score":55,"bear_score":60} -->'))
      .toBe("neutral");
    // 50 vs 61：61 > 60 ⇒ 看空（边界恰好越过 1.2 倍）
    expect(classifyAnalystReport('<!-- VERDICT: {"bull_score":50,"bear_score":61} -->'))
      .toBe("bearish");
  });

  it("无结构化分数时回退 classifySentiment 关键词判定", () => {
    expect(classifyAnalystReport("综合判断建议买入，后市看多。")).toBe("bullish");
    expect(classifyAnalystReport("建议卖出回避，趋势看空。")).toBe("bearish");
    expect(classifyAnalystReport("建议继续持有，震荡观望。")).toBe("neutral");
  });

  it("回归锁（2026-10 顶部条 10/10 vs 分歧卡 9/11）：无 VERDICT 包裹的整篇 JSON，分数压过矛盾 stance", () => {
    // 旧顶部条：classifySentiment 走 stance 优先 ⇒ 看多；
    // 分歧卡：裸分数正则 ⇒ 30 vs 70 看空。统一后以分歧卡口径为准。
    const report = '{"stance":"看多","bull_score":30,"bear_score":70}';
    expect(classifyAnalystReport(report)).toBe("bearish");
  });

  it("回归锁：VERDICT 分数 0-1 与 0-100 混排时先归一化再比阈值", () => {
    // 0.7（=70） vs 60：归一化后双方均未达 1.2 倍 ⇒ 中性；
    // 旧顶部条不归一化（0.7 vs 60）会误判看空。
    const report = '<!-- VERDICT: {"bull_score":0.7,"bear_score":60} -->';
    expect(classifyAnalystReport(report)).toBe("neutral");
  });
});

describe("tallyAnalystSentiment", () => {
  it("按统一口径逐份计票并给出 total", () => {
    const reports = [
      '<!-- VERDICT: {"bull_score":80,"bear_score":10} -->',
      '<!-- VERDICT: {"bull_score":10,"bear_score":80} -->',
      '<!-- VERDICT: {"bull_score":55,"bear_score":58} -->',
    ];
    expect(tallyAnalystSentiment(reports)).toEqual({
      bullish: 1,
      bearish: 1,
      neutral: 1,
      total: 3,
    });
  });

  it("空输入返回全 0", () => {
    expect(tallyAnalystSentiment([])).toEqual({ bullish: 0, bearish: 0, neutral: 0, total: 0 });
  });

  it("调用侧 cleanToolCallTags 后 tool_call 内的矛盾 JSON 不参与计票", () => {
    // 旧顶部条只剥 tool_call 标签、保留其中 JSON ⇒ 裸分数 95/5 会被判看多；
    // cleanToolCallTags 整块删除后只剩「卖出看空」正文 ⇒ 看空。
    const raw = '<tool_call>{"bull_score":95,"bear_score":5}</tool_call>综合来看建议卖出，看空。';
    expect(tallyAnalystSentiment([cleanToolCallTags(raw)])).toEqual({
      bullish: 0,
      bearish: 1,
      neutral: 0,
      total: 1,
    });
  });

  it("截图票型 9/11/4 与 10/10/4 计票结果差异可被同一份口径复现，且总数均为 24", () => {
    const reports: string[] = [];
    for (let i = 0; i < 9; i++) {
      reports.push('<!-- VERDICT: {"bull_score":80,"bear_score":10} -->');
    }
    for (let i = 0; i < 11; i++) {
      reports.push('<!-- VERDICT: {"bull_score":10,"bear_score":80} -->');
    }
    for (let i = 0; i < 4; i++) {
      reports.push('<!-- VERDICT: {"bull_score":55,"bear_score":58} -->');
    }
    const tally = tallyAnalystSentiment(reports);
    expect(tally).toMatchObject({ bullish: 9, bearish: 11, neutral: 4 });
    expect(tally.total).toBe(24);
  });
});

describe("deriveAnalystConsensus", () => {
  it("多空并存且均未达 65% → 分歧（截图两种票型都是分歧）", () => {
    expect(deriveAnalystConsensus({ bullish: 10, bearish: 10, neutral: 4, total: 24 }))
      .toBe("divided");
    expect(deriveAnalystConsensus({ bullish: 9, bearish: 11, neutral: 4, total: 24 }))
      .toBe("divided");
  });

  it("一方占比 > 65% → 单边共识", () => {
    expect(deriveAnalystConsensus({ bullish: 17, bearish: 3, neutral: 4, total: 24 }))
      .toBe("bullish");
    expect(deriveAnalystConsensus({ bullish: 1, bearish: 5, neutral: 0, total: 6 }))
      .toBe("bearish");
  });

  it("全中性 / 仅单边未达阈值 / 无票 → 中性", () => {
    expect(deriveAnalystConsensus({ bullish: 0, bearish: 0, neutral: 4, total: 4 }))
      .toBe("neutral");
    // 3/5 = 60%，未过 65%，且无空方票 ⇒ 中性（不构成分歧）
    expect(deriveAnalystConsensus({ bullish: 3, bearish: 0, neutral: 2, total: 5 }))
      .toBe("neutral");
    expect(deriveAnalystConsensus({ bullish: 0, bearish: 0, neutral: 0, total: 0 }))
      .toBe("neutral");
  });
});
