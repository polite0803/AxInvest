// SPDX-License-Identifier: AGPL-3.0-only

import type { StockDecision } from "@/types";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { DecisionTrustNotice } from "../DecisionTrustNotice";

// 用固定中文字典代替真实 i18n，便于断言文案（与 DecisionBanner.test.tsx 同风格，
// 但那里用 `fallback ?? key` 只能断言 key，本组件需要断言「被动降级」这类语义文案）。
vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, opts?: Record<string, unknown>) => {
      const dict: Record<string, string> = {
        "stockAnalysis.trustNotice.title": "决策可信度受限",
        "stockAnalysis.trustNotice.tagLabel": "可信度受限",
        "stockAnalysis.trustNotice.passiveWatch": "本次「观望」为数据不足导致的被动降级，非看空判断",
        "stockAnalysis.trustNotice.gapsNotDegraded": "存在数据缺口，但未触发降级：结论方向不受影响",
        "stockAnalysis.trustNotice.collapseLabel": "因子权重坍缩",
        "stockAnalysis.trustNotice.gapReason": `数据缺口 ${opts?.count ?? 0} 项`,
        "stockAnalysis.trustNotice.showGaps": "查看缺口",
        "stockAnalysis.trustNotice.hideGaps": "收起",
        "stockAnalysis.weightCollapseDqi": "数据质量 F 级",
        "stockAnalysis.weightCollapseUntrusted": `不可信上游 ${opts?.count} 个`,
        "stockAnalysis.weightCollapseThreshold": `权重占比 ${opts?.ratio}% 低于阈值`,
        "stockAnalysis.weightCollapseConsequence": "后果：仓位强制 0%、方向置信度 ×0.5",
      };
      return dict[key] ?? key;
    },
  }),
  initReactI18next: { type: "3rdParty", init: () => {} },
}));

/** 构造决策对象（默认是一个「健康」的买入决策，测试按需覆盖字段） */
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

describe("DecisionTrustNotice", () => {
  it("无权重坍缩且无数据缺口时完全不渲染（避免每张卡挂无信息量提示）", () => {
    const { container } = render(
      <DecisionTrustNotice decision={mkDecision({})} variant="banner" />,
    );
    expect(container.firstChild).toBeNull();
  });

  it("权重坍缩时渲染警示条与坍缩原因", () => {
    render(
      <DecisionTrustNotice
        decision={mkDecision({
          action: "WAIT",
          positionPct: 0,
          weightsCollapsed: true,
          collapseReason: "dqi_collapsed",
        })}
        variant="banner"
      />,
    );
    expect(screen.getByText(/决策可信度受限/)).toBeTruthy();
    expect(screen.getByText(/数据质量 F 级/)).toBeTruthy();
  });

  it("坍缩原因分档：multi_untrusted 带上游个数", () => {
    render(
      <DecisionTrustNotice
        decision={mkDecision({
          weightsCollapsed: true,
          collapseReason: "multi_untrusted",
          untrustedCount: 3,
        })}
        variant="banner"
      />,
    );
    expect(screen.getByText(/不可信上游 3 个/)).toBeTruthy();
  });

  it("坍缩原因缺省时退回「权重占比」档（low_weight_ratio）", () => {
    render(
      <DecisionTrustNotice
        decision={mkDecision({
          weightsCollapsed: true,
          collapseReason: "low_weight_ratio",
          weightRatio: 12.3,
        })}
        variant="banner"
      />,
    );
    expect(screen.getByText(/权重占比 12.3% 低于阈值/)).toBeTruthy();
  });

  // 2026-09-21 回归锁定（C 修复）：缺口 ≠ 降级。
  // 旧判据 `(collapsed || gaps.length > 0)` 只要有一项数据缺口就渲染「本次「观望」为
  // 数据不足导致的被动降级」—— 688114 实证：`collapseReason=none`（权重未坍缩）、
  // 三路 action 全「观望」、action 维度 30/30、仓位 8.4%，却仍被说成「被动降级」，
  // 与同一张卡片自身的数据直接打架。
  it("观望 + 缺口但权重未坍缩 → 不称「被动降级」，改称「未触发降级」", () => {
    render(
      <DecisionTrustNotice
        decision={mkDecision({
          action: "WAIT",
          positionPct: 8.4,
          weightsCollapsed: false,
          collapseReason: "none",
          dataGaps: ["PE数据(t-risk)"],
        })}
        variant="banner"
      />,
    );
    expect(screen.getByText(/决策可信度受限/)).toBeTruthy();
    expect(screen.getByText(/数据缺口 1 项/)).toBeTruthy();
    expect(screen.getByText(/存在数据缺口，但未触发降级/)).toBeTruthy();
    expect(screen.queryByText(/被动降级，非看空判断/)).toBeNull();
  });

  it("观望 + 缺口 + 权重坍缩 → 才标注「被动降级，非看空判断」", () => {
    render(
      <DecisionTrustNotice
        decision={mkDecision({
          action: "WAIT",
          positionPct: 0,
          weightsCollapsed: true,
          dataGaps: ["资金流向(t-hotmoney-data)", "公告数据(t-catalyst-data)"],
        })}
        variant="banner"
      />,
    );
    expect(screen.getByText(/被动降级，非看空判断/)).toBeTruthy();
    expect(screen.getByText(/数据缺口 2 项/)).toBeTruthy();
  });

  it("零仓位同样视为被动降级（仓位 0 的「持有」不成立）", () => {
    render(
      <DecisionTrustNotice
        decision={mkDecision({ action: "HOLD", positionPct: 0, weightsCollapsed: true })}
        variant="banner"
      />,
    );
    expect(screen.getByText(/被动降级，非看空判断/)).toBeTruthy();
  });

  it("有仓位时不标注被动降级（这是主动决策，不是降级）", () => {
    render(
      <DecisionTrustNotice
        decision={mkDecision({
          action: "BUY",
          positionPct: 25,
          dataGaps: ["龙虎榜数据(t-dragon-tiger-data)"],
        })}
        variant="banner"
      />,
    );
    expect(screen.getByText(/决策可信度受限/)).toBeTruthy();
    expect(screen.queryByText(/被动降级，非看空判断/)).toBeNull();
  });

  // 2026-09-12 回归锁定：卖出 / 减持天然零仓位（portfolio-mgr.rhai「action 决定了仓位性质」：
  // final_action == "卖出" → position_pct = 0），旧判据 `action === "WAIT" || positionPct <= 0`
  // 会把主动看空渲染成「本次「观望」为数据不足导致的被动降级，非看空判断」——
  // 与同一张卡片上的「卖出」标签直接打架，并且把方向说反。
  it("卖出 / 减持的零仓位不得标注被动降级（主动看空不是降级）", () => {
    for (const action of ["SELL", "REDUCE"] as const) {
      const { unmount } = render(
        <DecisionTrustNotice
          decision={mkDecision({
            action,
            positionPct: 0,
            dataGaps: ["PE数据(t-risk)"],
          })}
          variant="banner"
        />,
      );
      // 数据缺口本身仍须提示（可信度确实受限）
      expect(screen.getByText(/决策可信度受限/)).toBeTruthy();
      expect(screen.getByText(/数据缺口 1 项/)).toBeTruthy();
      // 但不得把主动看空说成「观望 / 非看空判断」
      expect(screen.queryByText(/被动降级，非看空判断/)).toBeNull();
      unmount();
    }
  });

  it("数据缺口默认收起，点击后可展开完整清单", async () => {
    const gaps = ["资金流向(t-hotmoney-data)", "公告数据(t-catalyst-data)"];
    render(
      <DecisionTrustNotice
        decision={mkDecision({ action: "WAIT", positionPct: 0, dataGaps: gaps })}
        variant="banner"
      />,
    );
    // 收起态：缺口明细不可见
    expect(screen.queryByText("资金流向(t-hotmoney-data)")).toBeNull();
    await userEvent.click(screen.getByText("查看缺口"));
    expect(screen.getByText("资金流向(t-hotmoney-data)")).toBeTruthy();
    expect(screen.getByText("公告数据(t-catalyst-data)")).toBeTruthy();
  });

  it("tag 形态渲染紧凑标签（简洁模式 / 工具栏用）", () => {
    render(
      <DecisionTrustNotice
        decision={mkDecision({ action: "WAIT", positionPct: 0, weightsCollapsed: true })}
        variant="tag"
      />,
    );
    expect(screen.getByText(/可信度受限/)).toBeTruthy();
    // tag 形态不铺开完整文案，只保留短标签
    expect(screen.queryByText(/被动降级，非看空判断/)).toBeNull();
  });
});
