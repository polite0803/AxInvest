// SPDX-License-Identifier: AGPL-3.0-only

import { describe, expect, it } from "vitest";

import { extractDecision, tryParseDecision } from "@/lib/agentOutput";
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
