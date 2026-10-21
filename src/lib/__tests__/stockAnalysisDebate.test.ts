/**
 * 辩论节点 id 的档位/轮次解析（v161 逐档辩论的前端配套）。
 *
 * 修复前的判据只有 `stepId.startsWith("bull-r")` + `parseInt`：
 * `bull-r1--ultra_short` / `--short` / `--mid` / `--long` 全被算成第 1 轮，
 * store 按 round 去重 ⇒ 后完成的档覆盖先完成的档，卡片上只剩一档文字。
 * 这里锁住新的分解判据（档位是身份的一部分，不能丢）。
 */
import { compareDebateRounds, debatePeerNodeId, parseDebateRoundRef } from "@/lib/stock-analysis-utils";
import { describe, expect, it } from "vitest";

describe("parseDebateRoundRef — 辩论节点 id 的档位/轮次分解", () => {
  it("v161+ 带档节点：档位是身份的一部分", () => {
    expect(parseDebateRoundRef("bull-r1--ultra_short")).toEqual({ tier: "ultra_short", round: 1 });
    expect(parseDebateRoundRef("bear-r2--long")).toEqual({ tier: "long", round: 2 });
  });

  it("历史单份与旧多轮节点 tier=null（回放兼容）", () => {
    expect(parseDebateRoundRef("bull-researcher")).toEqual({ tier: null, round: 1 });
    expect(parseDebateRoundRef("bear-r3")).toEqual({ tier: null, round: 3 });
  });

  it("非辩论 id / 未知档位 / 非法轮次一律 null，不得猜成第 1 轮", () => {
    expect(parseDebateRoundRef("a-market-analyst--mid")).toBeNull();
    expect(parseDebateRoundRef("debate-convergence--mid")).toBeNull();
    expect(parseDebateRoundRef("bull-r1--weekly")).toBeNull();
    expect(parseDebateRoundRef("bull-r0")).toBeNull();
    expect(parseDebateRoundRef("bull-researcher--mid")).toBeNull();
    expect(parseDebateRoundRef("bull-r1--")).toBeNull();
  });

  it("对侧 id 按同档同轮配对（不跨档取正文）", () => {
    expect(debatePeerNodeId("bull-r1--mid", "bear")).toBe("bear-r1--mid");
    expect(debatePeerNodeId("bear-r2--long", "bull")).toBe("bull-r2--long");
    expect(debatePeerNodeId("bull-researcher", "bear")).toBe("bear-researcher");
    expect(debatePeerNodeId("bull-r2", "bear")).toBe("bear-r2");
  });

  it("排序：四档按权威顺序，同档按轮次，历史单份排最后", () => {
    const rows = [
      { tier: null, round: 1 },
      { tier: "long", round: 1 },
      { tier: "ultra_short", round: 2 },
      { tier: "ultra_short", round: 1 },
      { tier: "mid", round: 1 },
    ];
    expect([...rows].sort(compareDebateRounds)).toEqual([
      { tier: "ultra_short", round: 1 },
      { tier: "ultra_short", round: 2 },
      { tier: "mid", round: 1 },
      { tier: "long", round: 1 },
      { tier: null, round: 1 },
    ]);
  });
});
