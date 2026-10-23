import { ACTION_PALETTE } from "@/lib/stock-analysis-utils";
import { describe, expect, it } from "vitest";
import { classifySentiment, getActionColor, getSignalColor, parseAction, StockAction } from "../stock-analysis";

describe("classifySentiment", () => {
  it('returns "bullish" for buy signals', () => {
    expect(classifySentiment("推荐买入该股票")).toBe("bullish");
    expect(classifySentiment("建议增持")).toBe("bullish");
    expect(classifySentiment("看多后市")).toBe("bullish");
    expect(classifySentiment("走势乐观，建议持有")).toBe("bullish");
  });

  it('returns "bearish" for sell signals', () => {
    expect(classifySentiment("建议卖出")).toBe("bearish");
    expect(classifySentiment("减持观望")).toBe("bearish");
    expect(classifySentiment("看空后市")).toBe("bearish");
    expect(classifySentiment("市场悲观")).toBe("bearish");
  });

  it('returns "neutral" for mixed or unclear signals', () => {
    expect(classifySentiment("市场走势平稳")).toBe("neutral");
    expect(classifySentiment("")).toBe("neutral");
    expect(classifySentiment("建议关注")).toBe("neutral");
  });

  it("reads stance field from JSON (trader: 买入/卖出/观望)", () => {
    expect(classifySentiment(JSON.stringify({ stance: "买入", positionPct: 35 }))).toBe("bullish");
    expect(classifySentiment(JSON.stringify({ stance: "卖出", positionPct: 0 }))).toBe("bearish");
    expect(classifySentiment(JSON.stringify({ stance: "观望", positionPct: 0 }))).toBe("neutral");
  });

  it("reads stance from news/debate agents (多头/空头/中性)", () => {
    expect(classifySentiment(JSON.stringify({ stance: "多头" }))).toBe("bullish");
    expect(classifySentiment(JSON.stringify({ stance: "空头" }))).toBe("bearish");
    expect(classifySentiment(JSON.stringify({ stance: "中性" }))).toBe("neutral");
  });

  it("reads bull_score / bear_score (0-10) — majority wins", () => {
    expect(classifySentiment(JSON.stringify({ bull_score: 6, bear_score: 4 }))).toBe("bullish");
    expect(classifySentiment(JSON.stringify({ bull_score: 3, bear_score: 7 }))).toBe("bearish");
    expect(classifySentiment(JSON.stringify({ bull_score: 5, bear_score: 5 }))).toBe("neutral");
  });

  it("reads positionPct (trader/debator) — ≥6 bullish, <0 bearish, else neutral", () => {
    expect(classifySentiment(JSON.stringify({ positionPct: 35 }))).toBe("bullish");
    expect(classifySentiment(JSON.stringify({ positionPct: 0 }))).toBe("neutral");
    expect(classifySentiment(JSON.stringify({ positionPct: 5 }))).toBe("neutral");
    expect(classifySentiment(JSON.stringify({ positionPct: -10 }))).toBe("bearish");
  });

  it("handles combined report JSON (stance + scores + free text)", () => {
    // 实际 LLM 输出：多头 + bull_score 6 + bear_score 4 + 自由文本里有风险提示
    const report = JSON.stringify({
      stance: "多头",
      bull_score: 6,
      bear_score: 4,
      summary: "技术面突破前高，基本面估值合理。短期存在回调风险，但中长期看好。",
    });
    expect(classifySentiment(report)).toBe("bullish");
  });

  it("text fallback: simple majority (not 65% threshold)", () => {
    // 旧 65% 阈值会让"看好 + 风险"被中性化；现在简单多数直接判看多
    expect(classifySentiment("看好后市，但短期有回调风险")).toBe("bullish");
    expect(classifySentiment("看空后市，但长期或有机会")).toBe("bearish");
  });
});

describe("parseAction", () => {
  it("parses standard actions", () => {
    expect(parseAction("买入")).toBe(StockAction.BUY);
    expect(parseAction("卖出")).toBe(StockAction.SELL);
    expect(parseAction("增持")).toBe(StockAction.INCREASE);
    expect(parseAction("减持")).toBe(StockAction.REDUCE);
    expect(parseAction("持有")).toBe(StockAction.HOLD);
    expect(parseAction("观望")).toBe(StockAction.WAIT);
  });

  it("falls back to UNCERTAIN for unknown actions", () => {
    // P1-1(2026-09-14)：未识别一律回 `UNCERTAIN`，**不是** `WAIT`。
    // 解析失败属于「无法判断」，不是业务语义上的「观望」——把缺失伪装成结论
    // 会让「系统没看懂」在界面上显示成「判断为中性」。见
    // `lib/stock-analysis-utils.ts::parseAction` 的文档注释。
    // 本用例此前期望 WAIT，属语义变更后未同步的滞后断言。
    expect(parseAction("unknown")).toBe(StockAction.UNCERTAIN);
    expect(parseAction("")).toBe(StockAction.UNCERTAIN);
    expect(parseAction(null)).toBe(StockAction.UNCERTAIN);
  });
});

describe("getSignalColor", () => {
  // 2026-10-23: 返回值不再是 antd 预设色名，而是**强度表端点的 hex**。写死 "red"/"green"
  //   等于把「红=利好」这条约定在两个文件里各表达一遍 —— 2026-09-21 那次「看多绿、买入红」
  //   就是这么来的。断言因此打在「与权威同端同色」上，色值再调整也不会假红。
  it("看多 / 买入 / 涨 / 牛 ⇒ 强度表最利好那一端", () => {
    for (const s of ["买入信号", "看多", "上涨趋势", "牛"]) {
      expect(getSignalColor(s)).toBe(getActionColor(StockAction.BUY));
    }
  });

  it("看空 / 卖出 / 跌 / 熊 ⇒ 强度表最利空那一端", () => {
    for (const s of ["卖出信号", "看空", "下跌趋势", "熊"]) {
      expect(getSignalColor(s)).toBe(getActionColor(StockAction.SELL));
    }
  });

  it("无方向关键词 ⇒ 中性档，且不得借用「缺失」档的色（缺席不是中性）", () => {
    expect(getSignalColor("关注")).toBe(getActionColor(StockAction.HOLD));
    expect(getSignalColor("")).toBe(getActionColor(StockAction.HOLD));
    expect(getSignalColor("")).not.toBe(getActionColor(StockAction.UNAVAILABLE));
  });

  it("与 getActionColor 同源：三端都落在强度表的 hex 上（防两套口径回流）", () => {
    const hexes = new Set(Object.values(ACTION_PALETTE).map((p) => p.hex));
    for (const s of ["看多", "看空", "关注"]) {
      expect(hexes.has(getSignalColor(s))).toBe(true);
    }
    expect(getSignalColor("看多")).toBe(getActionColor("买入"));
    expect(getSignalColor("看空")).toBe(getActionColor("卖出"));
  });
});
