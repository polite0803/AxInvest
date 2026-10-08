// SPDX-License-Identifier: AGPL-3.0-only

import type { StockDecision } from "@/types";
import { render, screen } from "@testing-library/react";
import * as fs from "node:fs";
import * as path from "node:path";
import { describe, expect, it, vi } from "vitest";
import { DecisionTrustNotice } from "../DecisionTrustNotice";

// 固定中文字典（与 DecisionTrustNotice.test.tsx 同风格）：断言的是「说的是哪件事」，
// 不是某个措辞字符串本身。
vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, opts?: Record<string, unknown>) => {
      const dict: Record<string, string> = {
        "stockAnalysis.trustNotice.title": "决策可信度受限",
        "stockAnalysis.trustNotice.tagLabel": "可信度受限",
        "stockAnalysis.trustNotice.passiveWatch": "被动降级",
        "stockAnalysis.trustNotice.gapsNotDegraded": "缺口未降级",
        "stockAnalysis.trustNotice.collapseLabel": "因子权重坍缩",
        "stockAnalysis.trustNotice.gapReason": `数据缺口 ${opts?.count ?? 0} 项`,
        "stockAnalysis.trustNotice.showGaps": "查看缺口",
        "stockAnalysis.trustNotice.hideGaps": "收起",
        "stockAnalysis.trustNotice.dqiFuse": `跨轮熔断(${opts?.streak ?? "?"})`,
        "stockAnalysis.weightCollapseThreshold": `权重占比 ${opts?.ratio}%`,
        "stockAnalysis.weightCollapseConsequence": "后果",
      };
      return dict[key] ?? key;
    },
  }),
  initReactI18next: { type: "3rdParty", init: () => {} },
}));

function mkDecision(over: Partial<StockDecision>): StockDecision {
  return {
    action: "BUY",
    positionPct: 20,
    targetPrice: null,
    stopLoss: null,
    reasoning: "",
    riskLevel: "MID",
    confidence: 60,
    ...over,
  };
}

describe("DecisionTrustNotice 跨轮熔断态（#8 P5）", () => {
  it("本轮一切正常、但处于熔断态 ⇒ 警示条必须出现并带上连续轮数", () => {
    // 关键：weightsCollapsed=false 且 dataGaps 为空 —— 这条卡片上**只有**跨轮熔断一件事。
    // 旧的可信度判据只看这两件事，于是"证据面一直坏"在界面上完全不显示。
    render(<DecisionTrustNotice decision={mkDecision({ dqiFuseState: "fused", dqiStreak: 4 })} />);
    expect(screen.getByText(/跨轮熔断\(4\)/)).toBeTruthy();
  });

  it("compact 形态在只有熔断时也渲染（旧判据下这条卡会整枚不出现）", () => {
    // ⚠ Tooltip 的正文是 rc-tooltip 懒渲染的（hover 才进 DOM），这里断言的是**标签本体**：
    //   它证明 tag 分支的 null 判据也带上了熔断态。正文文案由上面 banner 那条锁住。
    render(
      <DecisionTrustNotice
        decision={mkDecision({ dqiFuseState: "fused", dqiStreak: 3 })}
        variant="tag"
      />,
    );
    expect(screen.getByText(/可信度受限/)).toBeTruthy();
  });

  it("unobserved（观测表为空）不点亮警示条：那是设施没跑过，不是本次决策可信度受限", () => {
    const { container } = render(
      <DecisionTrustNotice decision={mkDecision({ dqiFuseState: "unobserved", dqiObservations: 0 })} />,
    );
    expect(container.textContent).toBe("");
  });

  it("ok 不点亮（负控：三态里只有 fused 算可信度问题）", () => {
    const { container } = render(
      <DecisionTrustNotice
        decision={mkDecision({ dqiFuseState: "ok", dqiStreak: 1, dqiObservations: 9 })}
      />,
    );
    expect(container.textContent).toBe("");
  });

  it("字段缺席（旧快照 / 产端没发）不点亮，也不塌成 fused", () => {
    const { container } = render(<DecisionTrustNotice decision={mkDecision({})} />);
    expect(container.textContent).toBe("");
  });
});

// ── 三载体反向锁 ──
// 熔断态的判据在 Rust（dao 常量），产端在 portfolio-mgr.rhai，消费端在本组件 + types +
// agentOutput 白名单，第四载体是 11 语言文案。任一处漏改，`tsc` 与 `cargo check` 都不会红
// —— 历史上同类缺陷（2026-09-11 `weightsCollapsed`）就是"字段一直发、界面从未显示"。

const ROOT = path.resolve(__dirname, "../../../..");
const RHAI = path.join(ROOT, "src-tauri/src/commands/portfolio-mgr.rhai");
const LANGS = ["ar", "de", "en-US", "es", "fr", "hi", "ja", "ko", "ru", "zh-CN", "zh-TW"];
const FUSE_KEY = "dqiFuse";
const ZH_KEY = ["zh-CN", "zh-TW"];

describe("熔断态三载体一致", () => {
  it("产端脚本真的输出了 TS 侧读的四个键", () => {
    const body = fs.readFileSync(RHAI, "utf8");
    for (
      const key of [
        "dqiFuseState",
        "dqiStreak",
        "dqiObservations",
        "confidenceQualityCap",
      ]
    ) {
      expect(body.includes(`"${key}"`), `portfolio-mgr.rhai 未输出 ${key}`).toBe(true);
    }
    // 三态值域必须与组件判据同集合：组件只认 "fused"，产端却要发三态
    for (const state of ["unobserved", "fused", "ok"]) {
      expect(body.includes(`"${state}"`), `产端缺三态值 ${state}`).toBe(true);
    }
  });

  it("normalizeDecision 是白名单重建 ⇒ 四个键必须显式带上（漏一个就是界面永远读不到）", () => {
    const src = fs.readFileSync(path.join(ROOT, "src/lib/agentOutput.ts"), "utf8");
    for (const key of [FUSE_KEY + "State", "dqiStreak", "dqiObservations", "confidenceQualityCap"]) {
      expect(src.includes(`${key}:`), `agentOutput.ts 未透传 ${key}`).toBe(true);
    }
  });

  it("11 语言全部真译：非空、含插值参数、非中文不得抄 zh-CN", () => {
    const zh = JSON.parse(
      fs.readFileSync(path.join(ROOT, "src/i18n/locales/zh-CN.json"), "utf8"),
    ).stockAnalysis.trustNotice[FUSE_KEY] as string;
    expect(zh).toContain("熔断");
    for (const lang of LANGS) {
      const file = path.join(ROOT, `src/i18n/locales/${lang}.json`);
      const value = JSON.parse(fs.readFileSync(file, "utf8")).stockAnalysis.trustNotice[FUSE_KEY];
      expect(typeof value, `${lang} 缺该 key`).toBe("string");
      expect((value as string).trim().length, `${lang} 空文案`).toBeGreaterThan(8);
      // 中文两语本身就是 zh-CN/zh-TW，不参与「不得抄 zh-CN」判定（否则恒红）；
      // 繁简两体是否真的分开了，由下面单独一条锁住。
      if (!ZH_KEY.includes(lang)) {
        expect(value, `${lang} 抄了 zh-CN`).not.toBe(zh);
      }
      // i18next 插值语法是双花括号；写成单花括号会原样显示 {streak} 而不报错
      expect(value, `${lang} 缺 {{streak}} 插值`).toContain("{{streak}}");
    }
    // 中文两语各自成句（zh-TW 不得与 zh-CN 逐字相同 —— 术语不同即不同）
    const tw = JSON.parse(
      fs.readFileSync(path.join(ROOT, "src/i18n/locales/zh-TW.json"), "utf8"),
    ).stockAnalysis.trustNotice[FUSE_KEY] as string;
    expect(tw).not.toBe(zh);
    expect(ZH_KEY.length).toBe(2);
  });
});
