// SPDX-License-Identifier: AGPL-3.0-only
//
// PLAN-mover-coverage-and-reflection-exit 阶段 3 / 决-3 的呈现层回归。
//
// 钉四件事（每一件都对应一个现网实测的坏形态）：
//   ① 四类层级计数各占一格，`unlabelled`（反思没判层级）**不得**折进 `l1` ——
//      面板旧实现把缺省写死 "L1"，于是现网 79/80 条「没判」被显示成「判成参数层」；
//   ② 结构层（L2/L3）必须有明细出口 —— 只把它们排除在参数网格之外而不给落点，
//      等于把根因藏得更深；
//   ③ 「已排除在网格外」这句只在该窗确有 L2/L3 时出现（全 L1 的看板不许喊这句）；
//   ④ `buildAiAssistPrompt` / `parseActionsFromAccumulated` 是结构层的最后一环：
//      前者不得伪造层级，后者必须同时认它自己声明的 snake_case 拼写
//      （旧实现只读 camelCase ⇒ 照 schema 输出的动作块被静默丢弃）。
// 每条各用一个破坏样本，且只让对应那一句红。

import { render } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { TierBoard } from "@/types";
import type { ReflectionRow } from "../ReflectionPanel";
import { buildAiAssistPrompt, parseActionsFromAccumulated, TierSummary } from "../ReflectionPanel";

/** t 替身：回显 key，插值对象拼成 `key|k=v`，断言即可定位到具体文案 key */
const t = (key: string, opts?: Record<string, unknown>): string =>
  opts ? `${key}|${Object.entries(opts).map(([k, v]) => `${k}=${v}`).join(",")}` : key;

function boardOf(over: Partial<TierBoard> = {}): TierBoard {
  return {
    totalRows: 80,
    l1: 0,
    l2: 0,
    l3: 0,
    unlabelled: 80,
    structuralRows: [],
    ...over,
  };
}

function textOf(board: TierBoard): string {
  const { container } = render(<TierSummary board={board} t={t} />);
  return container.textContent ?? "";
}

describe("TierSummary（阶段 3 层级四态不得互顶）", () => {
  it("未标注单独成句并点名「不等于参数层」", () => {
    const txt = textOf(boardOf({ unlabelled: 79, l1: 1 }));
    expect(txt).toContain("stockAnalysis.reflection.tierUnlabelledHint|count=79");
    // 四态数字各自可见（总数 / L1 / 未标注都不同值，收敛成一个数即红）
    expect(txt).toContain("79");
    expect(txt).toContain("1");
    // 本样本无 L2/L3 ⇒ 不得出现「已排除在网格外」那句（那是结构层专用的）
    expect(txt).not.toContain("stockAnalysis.reflection.tierGridNote");
  });

  it("结构层明细逐条渲染层级、标的、提案与错因", () => {
    const txt = textOf(
      boardOf({
        l1: 3,
        l2: 2,
        l3: 1,
        unlabelled: 0,
        structuralRows: [
          {
            reflectionId: "r-1",
            stockCode: "002696",
            stockName: "百洋股份",
            horizon: "ultra_short",
            tier: "L2",
            proposal: "portfolio-mgr-h-ultra-short.rhai 进门门加相对强度分支",
            whatWentWrong: "近 5 日未涨停 ⇒ 进门门否掉",
            updatedAt: 1_760_000_000_000,
          },
        ],
      }),
    );
    expect(txt).toContain("L2");
    expect(txt).toContain("002696");
    expect(txt).toContain("百洋股份");
    expect(txt).toContain("进门门加相对强度分支");
    expect(txt).toContain("近 5 日未涨停");
    // 档必须走本地化键（直出 ultra_short 就是前端自己拼后端值域）
    expect(txt).toContain("stockAnalysis.timeHorizon");
    expect(txt).toContain("stockAnalysis.reflection.tierGridNote");
  });

  it("档缺席显示破折号，不冒充某档", () => {
    const txt = textOf(
      boardOf({
        l2: 1,
        structuralRows: [
          {
            reflectionId: "r-2",
            stockCode: "600276",
            stockName: "恒瑞医药",
            horizon: null,
            tier: "L2",
            proposal: null,
            whatWentWrong: null,
            updatedAt: 1,
          },
        ],
      }),
    );
    expect(txt).not.toContain("stockAnalysis.timeHorizon");
    expect(txt).toContain("—");
  });

  it("无结构层时给「暂无待裁项」而不是空表（空表会被读成「组件坏了」）", () => {
    const txt = textOf(boardOf({ l1: 12, unlabelled: 0 }));
    expect(txt).toContain("stockAnalysis.reflection.tierEmpty");
  });
});

function rowOf(over: Partial<ReflectionRow> = {}): ReflectionRow {
  return {
    id: "r-1",
    stockCode: "002696",
    stockName: "百洋股份",
    asOfDate: "2026-09-28",
    hindsightDate: "2026-10-16",
    actualOutcome: "上涨 65%",
    whatWentWrong: "判观望，实际题材驱动连板",
    missedSignals: null,
    fixForFuture: "把相对强度接进证据面",
    reflectionDepth: "deep",
    minConfidenceThreshold: 60,
    status: "completed",
    createdAt: 1_760_000_000_000,
    ...over,
  };
}

describe("buildAiAssistPrompt（结构层出口：缺席不得冒充 L1）", () => {
  it("decision_json 里没有 implementation_tier 时，不得写成 L1", () => {
    const prompt = buildAiAssistPrompt(rowOf({ decisionJson: JSON.stringify({ verdict: "wrong" }) }));
    expect(prompt).not.toContain("【实现层级（implementation_tier）】L1");
    expect(prompt).toContain("未声明 implementation_tier");
    // 同时给出「按根因自行判定」的指令，而不是留一个假默认值让下游 AI 照抄
    expect(prompt).toContain("自行判定 L1/L2/L3");
  });

  it("模板产出 L2 时原样带上，并保留代码 diff 提案", () => {
    const prompt = buildAiAssistPrompt(
      rowOf({
        decisionJson: JSON.stringify({
          reflection: {
            implementation_tier: "L2",
            code_diff_proposal: "在 pm_ultra_entry_gate 增加相对强度分支",
            params_suggestion: [{ param: "buy_threshold", suggested_value: 0.62, reason: "x" }],
          },
        }),
      }),
    );
    expect(prompt).toContain("【实现层级（implementation_tier）】L2");
    expect(prompt).toContain("pm_ultra_entry_gate");
    expect(prompt).toContain("buy_threshold");
  });
});

describe("parseActionsFromAccumulated（动作块拼写两侧都得认）", () => {
  const block = (payload: string) => `说明\n:::action\n${payload}\n:::\n`;

  it("prompt 自己声明的 snake_case 必须被解析（旧实现只读 camelCase ⇒ 静默丢光）", () => {
    const actions = parseActionsFromAccumulated(
      block('{"action_type":"edit_asset_file","data":{"path":"a.rhai","description":"x"}}'),
    );
    expect(actions).toHaveLength(1);
    expect(actions[0].actionType).toBe("edit_asset_file");
  });

  it("模型改吐 camelCase 时同样接受", () => {
    const actions = parseActionsFromAccumulated(
      block('{"actionType":"update_variable","data":{"name":"buy_threshold","value":0.62}}'),
    );
    expect(actions).toHaveLength(1);
    expect(actions[0].actionType).toBe("update_variable");
  });

  it("两种拼写都缺、或值域外的 action 仍然丢弃（负控：不是来者不拒）", () => {
    expect(parseActionsFromAccumulated(block('{"data":{}}'))).toHaveLength(0);
    expect(parseActionsFromAccumulated(block('{"action_type":"drop_table","data":{}}'))).toHaveLength(
      0,
    );
  });
});
