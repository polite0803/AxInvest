import { render } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { LlmHorizonDecisionStrip } from "../LlmHorizonDecisionStrip";

// i18next 替身直接回吐 key 原文 ⇒ 四种缺席状态各占一句这件事只能按 key 断言（按中文断言会跟着文案改）
vi.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string) => key }),
  // `@/lib/stock-analysis-utils` 会经 browserMock 拉起 `src/i18n/index.ts`，
  // 那里要 `i18n.use(initReactI18next)` ⇒ 只 mock useTranslation 会整档失败（同 HorizonDecisionStrip.test）
  initReactI18next: { type: "3rdParty", init: () => {} },
}));

const TIERS = ["ultraShort", "short", "mid", "long"] as const;

const row = (action: string, positionPct: number) => ({
  action,
  verdict: "看多",
  positionPct,
  confidence: 60,
  stopLossPct: 5,
  takeProfitPct: 10,
  expectedHoldingDays: 5,
});

const blob = (obj: Record<string, unknown>) => JSON.stringify(obj);

const slots = (container: HTMLElement) =>
  TIERS.map((t) => container.querySelector(`[data-testid="llm-horizon-slot-${t}"]`));

describe("LlmHorizonDecisionStrip", () => {
  it("四档齐备：四个槽位都在且都标 given", () => {
    const json = blob({
      horizonDecisionContract: 1,
      decisions: {
        ultraShort: row("观望", 0),
        short: row("增持", 15),
        mid: row("买入", 25),
        long: row("持有", 20),
      },
    });
    const { container } = render(<LlmHorizonDecisionStrip llmDecisionJson={json} testId="llm-strip" />);
    const found = slots(container);
    expect(found.every(Boolean)).toBe(true);
    expect(found.map((s) => s?.getAttribute("data-given"))).toEqual(["true", "true", "true", "true"]);
    expect(container.textContent).toContain("stockAnalysis.llmHorizonOverview");
    expect(container.textContent).not.toContain("stockAnalysis.llmHorizonNotGiven");
  });

  it("漏两档：四个槽位仍齐出，缺的那两格说「未给出」（不得整格消失）", () => {
    const json = blob({ horizonDecisionContract: 1, decisions: { short: row("增持", 15), mid: row("买入", 25) } });
    const { container } = render(<LlmHorizonDecisionStrip llmDecisionJson={json} testId="llm-strip" />);
    const given = slots(container).map((s) => s?.getAttribute("data-given"));
    expect(given).toEqual(["false", "true", "true", "false"]);
    // 两格缺席 ⇒ 两句「未给出」，不是四格缩成两格
    const occurrences = (container.textContent ?? "").split("stockAnalysis.llmHorizonNotGiven").length - 1;
    expect(occurrences).toBe(2);
  });

  /**
   * 有代际标记、但整份 `decisions` 没给 ⇒ 呈现层是**四格「未给出」**（不是整条消失）。
   * 理由：标记在 = 本轮确实按逐档契约跑过，四档「都没给」是真结论；
   * 而「trader 违反契约」这一成因由缺口清单承担（Rust 侧那条汇总缺口），
   * 不在这里再开第五个状态 —— 否则同一个事实在两层各有说法。
   */
  it("整档没给：四格都「未给出」，与「上线前」那一态不同形", () => {
    const { container } = render(
      <LlmHorizonDecisionStrip
        llmDecisionJson={blob({ horizonDecisionContract: 1, action: "买入" })}
        testId="llm-strip"
      />,
    );
    expect(slots(container).map(() => true)).toEqual([true, true, true, true]);
    expect(slots(container).every((s) => s?.getAttribute("data-given") === "false")).toBe(true);
    expect(container.textContent).toContain("stockAnalysis.llmHorizonOverview");
    expect(container.textContent).not.toBe("stockAnalysis.llmHorizonPreContract");
  });

  it("代际标记缺席（上线前样本）：单独成句，且不渲染四格", () => {
    const { container } = render(
      <LlmHorizonDecisionStrip llmDecisionJson={blob({ action: "买入", verdict: "看多" })} testId="llm-strip" />,
    );
    expect(container.textContent).toBe("stockAnalysis.llmHorizonPreContract");
    expect(slots(container).some(Boolean)).toBe(false);
  });

  it("整列不可解析：单独成句，与「上线前」和「未给出」都不同形", () => {
    const { container } = render(
      <LlmHorizonDecisionStrip llmDecisionJson="这不是 JSON，只是模型吐的一段散文" testId="llm-strip" />,
    );
    expect(container.textContent).toBe("stockAnalysis.llmHorizonParseFailed");
  });

  it("整列为空：组件不渲染任何东西", () => {
    const { container } = render(<LlmHorizonDecisionStrip llmDecisionJson={null} testId="llm-strip" />);
    expect(container.innerHTML).toBe("");
  });

  /**
   * 负控（拆在「代际标记」这道守卫上，不是拆在夹具拼写上）：
   * 同一份 decisions 内容，去掉标记就必须判成「上线前」而不是「四档齐备」——
   * 若有人把判据反了（无标记也当 ok 读四档），上一条测试与本条会同时失效，所以两条都留着。
   */
  it("负控：有 decisions 但无代际标记 ⇒ 不得读成四档齐备", () => {
    const full = {
      decisions: {
        ultraShort: row("观望", 0),
        short: row("增持", 15),
        mid: row("买入", 25),
        long: row("持有", 20),
      },
    };
    const { container } = render(<LlmHorizonDecisionStrip llmDecisionJson={blob(full)} testId="llm-strip" />);
    expect(container.textContent).toBe("stockAnalysis.llmHorizonPreContract");
    expect(slots(container).some(Boolean)).toBe(false);
  });

  /**
   * 负控：读侧**不重做键归一**（第二份真相）。喂 snake 形态的逐档键 ⇒ 一格也不认。
   * 这是刻意选的 fail-closed：宁可显示「未给出」，也不让 UI 悄悄兜住后端的键名漂移；
   * 键名漂移由 `decision.rs` 的归一测试 + `check-horizon-decision-keys.mjs` 拦。
   */
  it("负控：snake 键不认（归一只在 Rust 侧做一次）", () => {
    const json = blob({
      horizonDecisionContract: 1,
      decisions: { ultra_short: row("观望", 0), mid: row("买入", 25) },
    });
    const { container } = render(<LlmHorizonDecisionStrip llmDecisionJson={json} testId="llm-strip" />);
    // 只有 camel 的 mid 被认；`ultra_short` 明明在 blob 里，也不许被读成 ultraShort 那一格
    expect(slots(container).map((s) => s?.getAttribute("data-given"))).toEqual(["false", "false", "true", "false"]);
  });
});
