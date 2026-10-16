// SPDX-License-Identifier: AGPL-3.0-only
//
// PLAN-stock-decision-reliability P2-b（失败模式结构化归因）的呈现层回归。
//
// 钉的是「三种缺席不得互相顶替」：
//   ① unlabelled>0（模板没产出该字段）必须单独成句，不能被空表格当成「没有失败模式」；
//   ② rejectedTokens（模型自报了词表外的名字）必须单独成句；
//   ③ cells 为空但有标注时，文案是「无一条命中词表」而不是「未标注」。
// 每条各用一个破坏样本，且只让对应那一句红 —— 三条全红的断言等于一条门。

import { render } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { FailureModeBoard } from "@/types";
import { FailureModesSummary } from "../ReflectionPanel";

/** t 替身：回显 key，插值对象拼成 `key|k=v`，断言即可定位到具体文案 key */
const t = (key: string, opts?: Record<string, unknown>): string =>
  opts ? `${key}|${Object.entries(opts).map(([k, v]) => `${k}=${v}`).join(",")}` : key;

function boardOf(over: Partial<FailureModeBoard> = {}): FailureModeBoard {
  return {
    totalRows: 40,
    labelled: 0,
    unlabelled: 40,
    cells: [],
    rejectedTokens: [],
    ...over,
  };
}

function textOf(board: FailureModeBoard): string {
  const { container } = render(<FailureModesSummary board={board} t={t} />);
  return container.textContent ?? "";
}

describe("FailureModesSummary（P2-b 三种缺席各占一句）", () => {
  it("未标注时给出条数并明写「模板未产出」，不得只留一张空表", () => {
    const txt = textOf(boardOf({ unlabelled: 12, labelled: 28 }));
    expect(txt).toContain("stockAnalysis.reflection.failureModesUnlabelledHint|count=12");
    // 三态计数字面出现（不得被收敛成一个数）
    expect(txt).toContain("40");
    expect(txt).toContain("28");
    expect(txt).toContain("12");
    // 该样本没有词表外值 ⇒ 不得出现 rejected 文案
    expect(txt).not.toContain("failureModesRejected");
  });

  it("库里存在词表外归因时单独成句并列出原值", () => {
    const txt = textOf(
      boardOf({
        unlabelled: 0,
        labelled: 5,
        cells: [{ mode: "chase_big_up_bar", count: 5, share: 1 }],
        rejectedTokens: ["sentiment_over_signal"],
      }),
    );
    expect(txt).toContain("failureModesRejected|tokens=sentiment_over_signal");
    // 该样本没有未标注行 ⇒ 未标注句不得出现（否则两种缺席混在一起）
    expect(txt).not.toContain("failureModesUnlabelledHint");
  });

  it("有标注但一条都没命中词表时，文案是「无命中」而不是「未标注」", () => {
    const txt = textOf(
      boardOf({ unlabelled: 0, labelled: 3, cells: [], rejectedTokens: ["invented_one"] }),
    );
    expect(txt).toContain("stockAnalysis.reflection.failureModesEmpty");
    expect(txt).not.toContain("failureModesUnlabelledHint");
  });

  it("命中格按词表键取译名，占比分母是已标注行", () => {
    const txt = textOf(
      boardOf({
        unlabelled: 0,
        labelled: 4,
        cells: [
          { mode: "chase_big_up_bar", count: 3, share: 0.75 },
          { mode: "reverse_dcf_ignored", count: 1, share: 0.25 },
        ],
      }),
    );
    expect(txt).toContain("stockAnalysis.reflection.failureModeChaseUpBar");
    expect(txt).toContain("stockAnalysis.reflection.failureModeReverseDcfIgnored");
    expect(txt).toContain("75.0%");
    expect(txt).toContain("25.0%");
  });

  it("词表外的模式值直出原串，面板不伪造译名", () => {
    const txt = textOf(
      boardOf({
        unlabelled: 0,
        labelled: 1,
        cells: [{ mode: "not_in_vocabulary", count: 1, share: 1 }],
      }),
    );
    expect(txt).toContain("not_in_vocabulary");
    expect(txt).not.toContain("stockAnalysis.reflection.failureModeOther");
  });
});
