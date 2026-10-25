import { normalizeDecision } from "@/lib/agentOutput";
import { describe, expect, it } from "vitest";

/**
 * v180：独立公式对照快照的白名单透传。
 *
 * `normalizeDecision` 是**白名单式重建** —— 产端在 decision_json 顶层加了字段、
 * 类型也声明了，这里漏一行就恒 `undefined`，面板于是「字段一直发、界面从未显示」
 * （2026-09-11 `weightsCollapsed` 那一族的复发形态）。本文件钉的是**透传这一环**：
 * 组件测试拿的是手搓 props，接不上这一环照样全绿。
 */
const base = {
  action: "BUY",
  confidence: 72,
  positionPct: 15,
  reasoning: "占位",
};

describe("normalizeDecision · f7ReferenceFormula / f7Source 透传（v183）", () => {
  it("齐备态：整块按原值带过去，中文字段不退化成 null", () => {
    const d = normalizeDecision({
      ...base,
      f7ReferenceFormula: {
        action: "买入",
        positionPct: 12.5,
        confidence: 68,
        posterior: 61.5,
        riskLevel: "中风险",
        f7Source: "risk_reward",
        f7WeightPct: 9.2,
        weightsCollapsed: false,
        dataGaps: ["龙虎榜"],
      },
    });
    expect(d?.f7ReferenceFormula).toEqual({
      action: "买入",
      positionPct: 12.5,
      confidence: 68,
      posterior: 61.5,
      riskLevel: "中风险",
      f7Source: "risk_reward",
      f7WeightPct: 9.2,
      weightsCollapsed: false,
      dataGaps: ["龙虎榜"],
    });
  });

  /**
   * 「该记录没跑本侧」与「跑了但内容为空」是两件事：前者 `undefined`、后者对象在场全 null。
   * 塌成同一个值，面板就会把「上线前的记录」显示成「跑了但全缺」。
   */
  it("键不存在 ⇒ undefined，不是空对象", () => {
    const d = normalizeDecision({ ...base });
    expect(d?.f7ReferenceFormula).toBeUndefined();
  });

  it("发了一个空对象 ⇒ 对象在场、内层全部缺席，不得被压成 undefined", () => {
    const d = normalizeDecision({ ...base, f7ReferenceFormula: {} });
    expect(d?.f7ReferenceFormula).toEqual({
      action: null,
      positionPct: null,
      confidence: null,
      posterior: null,
      riskLevel: null,
      f7Source: null,
      f7WeightPct: null,
      weightsCollapsed: null,
      dataGaps: null,
    });
  });

  it("脏值（字符串 / 数组）按未产出处理，不把脏值当快照", () => {
    expect(normalizeDecision({ ...base, f7ReferenceFormula: "risk_reward" })?.f7ReferenceFormula).toBeUndefined();
    expect(normalizeDecision({ ...base, f7ReferenceFormula: [] })?.f7ReferenceFormula).toBeUndefined();
  });

  it("数值槽的脏字符串回 null，不塌成 0（0 会被读成「本侧算出了 0 分」）", () => {
    const d = normalizeDecision({ ...base, f7ReferenceFormula: { confidence: "高", f7WeightPct: null } });
    expect(d?.f7ReferenceFormula?.confidence).toBeNull();
    expect(d?.f7ReferenceFormula?.f7WeightPct).toBeNull();
    expect(d?.f7ReferenceFormula?.positionPct).toBeNull();
  });

  it("方向档的空串/空白 ⇒ null（空串不是「一个合法档位」）", () => {
    const d = normalizeDecision({ ...base, f7ReferenceFormula: { action: "   " } });
    expect(d?.f7ReferenceFormula?.action).toBeNull();
  });
});

/**
 * v183：主决策**自己的** f7 口径也必须过白名单。
 * 面板要靠它说「本条到底读没读 LLM 决策」；漏透传时组件会退回 `unknown` 那句，
 * 而那句读起来像「产端没声明」，实际是「前端没接」—— 两类缺席混成一句话就是歧义。
 */
describe("normalizeDecision · f7Source 透传", () => {
  const base = { action: "BUY", confidence: 72, positionPct: 15, reasoning: "占位" };

  it("snake_case（现网 rhai 顶层键）与 camelCase 都收", () => {
    expect(normalizeDecision({ ...base, f7_source: "analyst_votes" })?.f7Source).toBe("analyst_votes");
    expect(normalizeDecision({ ...base, f7Source: "trader" })?.f7Source).toBe("trader");
  });

  it("键缺席 ⇒ undefined（不是 `absent`）：「记录早于 v183」与「本侧确实没输入」是两件事", () => {
    expect(normalizeDecision({ ...base })?.f7Source).toBeUndefined();
    expect(normalizeDecision({ ...base, f7_source: "absent" })?.f7Source).toBe("absent");
  });

  it("空串/空白 ⇒ undefined，不得顶成一个合法口径", () => {
    expect(normalizeDecision({ ...base, f7_source: "  " })?.f7Source).toBeUndefined();
  });
});
