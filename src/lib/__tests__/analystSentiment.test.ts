// i18n-exempt: 计票口径单测，断言全部为结构化输入/输出，无 UI 文案。
import { cleanToolCallTags } from "@/lib/agentOutput";
import {
  classifyAnalystReport,
  computeStockConsensus,
  deriveAnalystConsensus,
  extractBullBearScores,
  groupAnalystReportsByTier,
  scopeAnalystReports,
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

  it("不得把辩手的 strength_score 当成 bull_score（否则看空辩手被判成看多）", () => {
    // 辩手的 `strength_score` 是「立场强度」，不分多空。旧实现 `bull_score ?? strength_score`
    // 会把一个 bearish、strength=70 的辩手读成「看多 70 / 看空 0」，方向直接反转。
    const report = '<!-- VERDICT: {"stance":"bearish","strength_score":70} -->';
    expect(extractBullBearScores(report)).toEqual({ bull: 0, bear: 60 });
    expect(classifyAnalystReport(report)).toBe("bearish");
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

describe("groupAnalystReportsByTier", () => {
  const bullish = '<!-- VERDICT: {"bull_score":80,"bear_score":20} -->';
  const bearish = '<!-- VERDICT: {"bull_score":20,"bear_score":80} -->';

  it("逐档实例按档分组（每档自己的分析师子集）", () => {
    const reports = {
      "market-analyst--ultra_short": bearish,
      "market-analyst--mid": bullish,
      "policy--long": bullish,
    };
    expect(groupAnalystReportsByTier(reports)).toEqual({
      tiers: {
        ultra_short: { "market-analyst": bearish },
        mid: { "market-analyst": bullish },
        long: { policy: bullish },
      },
      untiered: {},
    });
  });

  it("裸键单独成组，不并入任何一档（分档前的历史快照）", () => {
    const reports = { "market-analyst": bearish, "investment-plan": bullish };
    expect(groupAnalystReportsByTier(reports)).toEqual({ tiers: {}, untiered: reports });
  });
});

describe("scopeAnalystReports", () => {
  const bullish = '<!-- VERDICT: {"bull_score":80,"bear_score":20} -->';
  const bearish = '<!-- VERDICT: {"bull_score":20,"bear_score":80} -->';
  const reports = {
    "market-analyst--ultra_short": bearish,
    "market-analyst--mid": bullish,
    "policy--long": bullish,
    "investment-plan": bullish,
  };

  it("该档不存在时回退 mid，不退回裸键", () => {
    // 请求 `short`，本次只跑出 ultra_short/mid/long ⇒ 回退 mid；
    // untiered 里的 investment-plan 不得被当成「short 档的分析师」。
    expect(scopeAnalystReports(reports, "short")).toEqual({ "market-analyst": bullish });
    expect(scopeAnalystReports(reports, "mid")).toEqual({ "market-analyst": bullish });
  });

  it("完全没有分档数据时才用裸键（历史快照）", () => {
    const legacy = { "market-analyst": bearish, "investment-plan": bullish };
    expect(scopeAnalystReports(legacy, "mid")).toEqual(legacy);
  });

  it("未指定档位时 mid 优先，其次唯一档", () => {
    expect(scopeAnalystReports(reports)).toEqual({ "market-analyst": bullish });
    expect(scopeAnalystReports({ "market-analyst--short": bearish })).toEqual({
      "market-analyst": bearish,
    });
  });
});

describe("computeStockConsensus", () => {
  const bullish = '<!-- VERDICT: {"bull_score":80,"bear_score":20} -->';
  const bearish = '<!-- VERDICT: {"bull_score":20,"bear_score":80} -->';

  it("逐档实例不重复计票：同一位分析师四档只算一票", () => {
    const reports = {
      "market-analyst--ultra_short": bullish,
      "market-analyst--short": bullish,
      "market-analyst--mid": bullish,
      "market-analyst--long": bullish,
    };
    const c = computeStockConsensus(reports, undefined, "mid");
    expect(c.votes.total).toBe(1);
    expect(c.votes.bullish).toBe(1);
    expect(c.bullish).toBe(1);
    expect(c.consensus).toBe("bullish");
  });

  it("逐档权重真的生效：超短档 hot-money(2.0)+market-analyst(1.3) ⇒ 加权和 3.3、票数 2", () => {
    // 负控：若查表仍按未剥档 / 未补 `a-` 的键名匹配，两者都会退 default 1.0，加权和会是 2。
    const reports = {
      "market-analyst--ultra_short": bullish,
      "hot-money--ultra_short": bullish,
    };
    const c = computeStockConsensus(reports, undefined, "ultra_short");
    expect(c.total).toBeCloseTo(3.3, 5);
    expect(c.bullish).toBeCloseTo(3.3, 5);
    // 两个计数域各自独立：加权和 3.3 不是票数，票数恒为整数 2
    expect(c.votes).toEqual({ bullish: 2, bearish: 0, neutral: 0, total: 2 });
  });

  it("共识标签只由票数决定，不再存在第二条加权判据", () => {
    // 长档权重：value-investor 2.0 + fundamentals 1.5 = 看多 3.5；
    //           market-analyst 0.6 + sentiment 0.7 = 看空 1.3。
    // 旧的第二条判据「净差 > 总权重 × 0.3」⇒ 2.2 > 1.44 ⇒ 标签=看多；
    // 票数口径（2 对 2，均未过 65%）⇒ 标签=分歧。
    // 两者曾在同一屏并存（顶部条走票数、交叉验证走加权），这就是那条同名不同值。
    const reports = {
      "value-investor--long": bullish,
      "fundamentals--long": bullish,
      "market-analyst--long": bearish,
      "sentiment--long": bearish,
    };
    const c = computeStockConsensus(reports, undefined, "long");
    expect(c.consensus).toBe("divided");
    expect(c.votes).toEqual({ bullish: 2, bearish: 2, neutral: 0, total: 4 });
    // 加权和仍透出供与后端证据报告对照，但它不再决定标签
    expect(c.bullish).toBeCloseTo(3.5, 5);
    expect(c.bearish).toBeCloseTo(1.3, 5);
    expect(c.total).toBeCloseTo(4.8, 5);
  });
});
