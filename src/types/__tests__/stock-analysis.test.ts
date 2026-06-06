import { describe, expect, it } from "vitest";
import { classifySentiment, getSignalColor, parseAction, StockAction } from "../stock-analysis";

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
});

describe("parseAction", () => {
  it("parses standard actions", () => {
    expect(parseAction("买入")).toBe(StockAction.BUY);
    expect(parseAction("卖出")).toBe(StockAction.SELL);
    expect(parseAction("增持")).toBe(StockAction.INCREASE);
    expect(parseAction("减持")).toBe(StockAction.REDUCE);
    expect(parseAction("持有")).toBe(StockAction.HOLD);
  });

  it("falls back to HOLD for unknown actions", () => {
    expect(parseAction("unknown")).toBe(StockAction.HOLD);
    expect(parseAction("")).toBe(StockAction.HOLD);
    expect(parseAction(null)).toBe(StockAction.HOLD);
  });
});

describe("getSignalColor", () => {
  it('returns "green" for buy/bull signals', () => {
    expect(getSignalColor("买入信号")).toBe("green");
    expect(getSignalColor("看多")).toBe("green");
    expect(getSignalColor("上涨趋势")).toBe("green");
  });

  it('returns "red" for sell/bear signals', () => {
    expect(getSignalColor("卖出信号")).toBe("red");
    expect(getSignalColor("看空")).toBe("red");
    expect(getSignalColor("下跌趋势")).toBe("red");
  });

  it('returns "blue" for neutral signals', () => {
    expect(getSignalColor("关注")).toBe("blue");
    expect(getSignalColor("")).toBe("blue");
  });
});
