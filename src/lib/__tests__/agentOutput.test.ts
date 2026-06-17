// SPDX-License-Identifier: AGPL-3.0-only

import { describe, expect, it } from "vitest";

import { cleanToolCallTags, extractDecision, tryParseDecision } from "@/lib/agentOutput";
import type { StockDecision } from "@/types/stock-analysis";

describe("agentOutput decision parsing", () => {
  const expectedDecision: StockDecision = {
    action: "BUY",
    positionPct: 20,
    targetPrice: null,
    stopLoss: null,
    reasoning: "Test decision",
    riskLevel: "MID",
    confidence: 85,
    timeHorizon: null,
    expectedHoldingDays: null,
    targetTimeframe: null,
  };

  it("parses plain JSON string into StockDecision", () => {
    const parsed = tryParseDecision('{"action":"BUY","positionPct":20,"confidence":85,"reasoning":"Test decision"}');
    expect(parsed).toEqual(expectedDecision);
  });

  it("parses JSON decision wrapped in markdown code block", () => {
    const parsed = tryParseDecision(
      `Here is the decision:\n\n\`\`\`json\n{\n  "action": "BUY",\n  "positionPct": 20,\n  "confidence": 85,\n  "reasoning": "Test decision"\n}\n\`\`\``,
    );
    expect(parsed).toEqual(expectedDecision);
  });

  it("parses escaped JSON string output", () => {
    const escaped = '"{"action":"BUY","positionPct":20,"confidence":85,"reasoning":"Test decision"}"';
    const parsed = tryParseDecision(escaped);
    expect(parsed).toEqual(expectedDecision);
  });

  it("returns null for JSON arrays in raw string output", () => {
    const parsed = tryParseDecision('[{"action":"BUY","positionPct":20,"confidence":85}]');
    expect(parsed).toBeNull();
  });

  it("extractDecision returns null for array object values", () => {
    const parsed = extractDecision([{ action: "BUY" }] as unknown);
    expect(parsed).toBeNull();
  });
});

describe("cleanToolCallTags", () => {
  it("removes generic Hermes/Qwen-style <tool_call> blocks with <function> and <parameter>", () => {
    const input = '<tool_call> <function=search_stock> <parameter=stock_code> 301302 </parameter> </function> </tool_call>';
    expect(cleanToolCallTags(input)).toBe("");
  });

  it("preserves surrounding text when tool_call is embedded", () => {
    const input = '分析完成。<tool_call> <function=search_stock> <parameter=stock_code> 301302 </parameter> </function> </tool_call>该股票基本面良好。';
    const cleaned = cleanToolCallTags(input);
    expect(cleaned).toBe("分析完成。该股票基本面良好。");
  });

  it("removes multiple tool_call blocks", () => {
    const input = '<tool_call><function=foo><parameter=x>1</parameter></function></tool_call>中间文本<tool_call><function=bar><parameter=y>2</parameter></function></tool_call>';
    const cleaned = cleanToolCallTags(input);
    expect(cleaned).toBe("中间文本");
  });

  it("removes orphan <function> and <parameter> tags without outer tool_call", () => {
    const input = '<function=search_stock><parameter=stock_code>301302</parameter></function>';
    expect(cleanToolCallTags(input)).toBe("");
  });

  it("removes tool_call with attributes", () => {
    const input = '<tool_call id="tc1"><function=search_stock><parameter=stock_code>301302</parameter></function></tool_call>';
    expect(cleanToolCallTags(input)).toBe("");
  });

  it("still handles provider-prefixed XML tool_call format", () => {
    const input = '<anthropic:tool_call>{"name":"search_stock"}</anthropic:tool_call>实际分析内容';
    expect(cleanToolCallTags(input)).toBe("实际分析内容");
  });

  it("does not match HTML-like tags such as <figure> or <param>", () => {
    const input = "<figure>图表说明</figure><param name=\"x\" value=\"1\">";
    expect(cleanToolCallTags(input)).toBe(input);
  });
});
