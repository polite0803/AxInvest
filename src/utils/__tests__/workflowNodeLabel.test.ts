import { HORIZON_T_SUFFIX } from "@/lib/stock-analysis-utils";
import type { TFunction } from "i18next";
import { describe, expect, it } from "vitest";
import { getWorkflowNodeLabel } from "../workflowNodeLabel";

/**
 * v128（B1）四个逐档风险节点在聊天卡片里的标签门。
 *
 * 要挡住的形态：四档节点全部退回裸 id（`cls-risk-level-mid`）或四格同名 ——
 * 那等于「按档」在呈现层又消失一次（用户裁定：UI 相关位置必须有四周期输出）。
 *
 * 判据不手抄档位清单：遍历值域权威 `HORIZON_T_SUFFIX`（与 DecisionBanner / 历史列表同一张表），
 * 将来给值域加第五档而忘了标签表，本门当场红。
 */
// 忠实模拟 i18next 的两态：在册 key 出译文，缺 key 走 defaultValue（不是「永远用 defaultValue」——
// 那样全局节点这一格会假绿，因为它本来就靠 defaultValue 兜底）
const KNOWN_KEYS = new Set([
  "stockAnalysis.workflow.riskLevel",
  ...Object.values(HORIZON_T_SUFFIX).map((s) => `stockAnalysis.timeHorizon${s}`),
]);
const fakeT =
  ((key: string, options?: { defaultValue?: string }) =>
    KNOWN_KEYS.has(key) ? `t:${key}` : options?.defaultValue ?? key) as unknown as TFunction;

function tierNodeId(snake: string): string {
  return `cls-risk-level-${snake.replace("_", "-")}`;
}

describe("getWorkflowNodeLabel（逐档风险节点）", () => {
  it("值域里每一档都有独立标签：风险等级分类 · 该档名", () => {
    const labels = Object.keys(HORIZON_T_SUFFIX).map((snake) => getWorkflowNodeLabel(tierNodeId(snake), fakeT));
    for (const label of labels) {
      expect(label).toContain("t:stockAnalysis.workflow.riskLevel");
      expect(label).toMatch(/^t:stockAnalysis\.workflow\.riskLevel · t:stockAnalysis\.timeHorizon/);
    }
    // 四档互不相同（同名等于没按档）
    expect(new Set(labels).size).toBe(Object.keys(HORIZON_T_SUFFIX).length);
  });

  it("全局节点仍走原标签，不带档位后缀", () => {
    expect(getWorkflowNodeLabel("cls-risk-level", fakeT)).toBe(
      "t:stockAnalysis.workflow.riskLevel",
    );
  });

  it("负控：未知后缀不得凭空拼出一个档位名，必须落回裸 id", () => {
    expect(getWorkflowNodeLabel("cls-risk-level-nano", fakeT)).toBe("cls-risk-level-nano");
    expect(getWorkflowNodeLabel("cls-risk-level-", fakeT)).toBe("cls-risk-level-");
  });
});
