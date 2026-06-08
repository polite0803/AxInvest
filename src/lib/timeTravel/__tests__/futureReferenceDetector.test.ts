import { describe, expect, it } from "vitest";
import { detectFutureReferences, detectFutureReferencesForNode } from "../futureReferenceDetector";

describe("detectFutureReferences — stage A (absolute dates)", () => {
  it("flags a future date string after as_of", () => {
    const hits = detectFutureReferences(
      "Market will close at 2026-07-01.",
      "2026-06-01",
    );
    expect(hits).toEqual([
      { snippet: "2026-07-01", ruleHit: "absolute-date" },
    ]);
  });

  it("ignores dates on or before as_of", () => {
    const hits = detectFutureReferences(
      "Past events: 2025-12-01 and 2026-06-01.",
      "2026-06-01",
    );
    expect(hits).toEqual([]);
  });

  it("dedups repeated snippets", () => {
    const hits = detectFutureReferences(
      "First 2027-01-01, second 2027-01-01 again.",
      "2026-06-01",
    );
    expect(hits).toEqual([
      { snippet: "2027-01-01", ruleHit: "absolute-date" },
    ]);
  });

  it("returns nothing when as_of is null (live mode)", () => {
    const hits = detectFutureReferences(
      "2027-01-01 tomorrow next week",
      null,
    );
    expect(hits).toEqual([]);
  });
});

describe("detectFutureReferences — stage B (tense phrases)", () => {
  it("flags 'tomorrow'", () => {
    const hits = detectFutureReferences(
      "The market will rally tomorrow.",
      "2026-06-01",
    );
    expect(hits).toContainEqual({
      snippet: "tomorrow",
      ruleHit: "tense-phrase",
    });
  });

  it("flags 'next quarter'", () => {
    const hits = detectFutureReferences(
      "Expect gains next quarter.",
      "2026-06-01",
    );
    expect(hits).toContainEqual({
      snippet: "next quarter",
      ruleHit: "tense-phrase",
    });
  });
});

describe("detectFutureReferences — stage C (vague future)", () => {
  it("flags 'soon'", () => {
    const hits = detectFutureReferences(
      "A recovery is coming soon.",
      "2026-06-01",
    );
    expect(hits).toContainEqual({
      snippet: "soon",
      ruleHit: "vague-future",
    });
  });
});

describe("detectFutureReferences — multi-stage combination", () => {
  it("captures all three stages in one text", () => {
    const text = "Expect 2026-07-01 rally tomorrow, soon.";
    const hits = detectFutureReferences(text, "2026-06-01");
    const rules = hits.map((h) => h.ruleHit);
    expect(rules).toContain("absolute-date");
    expect(rules).toContain("tense-phrase");
    expect(rules).toContain("vague-future");
  });

  it("returns empty on past-only text", () => {
    const hits = detectFutureReferences(
      "The earnings call on 2026-05-15 was a non-event.",
      "2026-06-01",
    );
    expect(hits).toEqual([]);
  });
});

describe("detectFutureReferencesForNode", () => {
  it("attaches the nodeId to each hit", () => {
    const hits = detectFutureReferencesForNode(
      "node-1",
      "Rally on 2027-01-01.",
      "2026-06-01",
    );
    expect(hits).toEqual([
      { nodeId: "node-1", snippet: "2027-01-01", ruleHit: "absolute-date" },
    ]);
  });
});
