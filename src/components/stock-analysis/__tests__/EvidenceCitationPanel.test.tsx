// 回归（2026-09-28）两件事：
// ① 决策未产出时，证据引用审计必须显式声明「不适用」，不得把「没有可审计对象」
//    渲染成「数据支撑率 0%」；
// ② 面板打开只发**一次** `extract_evidence_citations`。
// ⚠ 这里的 `t` 刻意**每次渲染新建**，还原 react-i18next 的真实形态：修复前组件把 `t`
//   放进 loadCitations 的 useCallback 依赖 ⇒ effect 每轮渲染重跑，实测同一渲染周期内
//   该 IPC 被调 38 次（下面用例把它锁回 1 次）。
import { render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("react-i18next", () => ({
  initReactI18next: { type: "3rdParty", init: () => {} },
  useTranslation: () => ({ t: (key: string) => key }),
}));

const invokeMock = vi.fn();
vi.mock("@/lib/invoke", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

import { EvidenceCitationPanel } from "../EvidenceCitationPanel";

const BASE = {
  stockCode: "600887",
  stockName: "测试标的",
  analysisDate: "2026-09-28",
  decisionAction: "数据缺失",
  decisionConfidence: 0,
  supportedClaims: 0,
  totalClaims: 0,
  supportRate: 0,
  analystCount: 0,
};

beforeEach(() => invokeMock.mockReset());

describe("EvidenceCitationPanel 决策未产出态", () => {
  it("decisionDegraded=true 时显示不适用提示，且不出现 0% 支撑率读数", async () => {
    // DB 实测形态：降级路径的异常诊断串被写进 reasoning，后端已拦下不再送匹配
    invokeMock.mockResolvedValue({
      ...BASE,
      decisionDegraded: true,
      citations: [],
    });
    render(<EvidenceCitationPanel analysisId="a-1" />);

    expect(await screen.findByText("stockAnalysis.evidenceCitation.notApplicable")).toBeTruthy();
    expect(screen.queryByText("0%")).toBeNull();
    expect(screen.queryByText("stockAnalysis.evidenceCitation.supportRate")).toBeNull();
  });

  it("对照：正常决策下「理由无一被支撑」仍如实读出 0%", async () => {
    invokeMock.mockResolvedValue({
      ...BASE,
      decisionAction: "减持",
      decisionDegraded: false,
      totalClaims: 1,
      citations: [
        {
          claim: "行业景气度向上",
          sourceAnalystId: "a-sector",
          sourceAnalystName: "行业分析师",
          matchConfidence: 0.4,
          sourceSnippet: "白酒行业整体营收增长15%",
          hasDataSupport: false,
          dataSource: null,
        },
      ],
    });
    render(<EvidenceCitationPanel analysisId="a-2" />);

    expect(await screen.findByText("0%")).toBeTruthy();
    expect(screen.queryByText("stockAnalysis.evidenceCitation.notApplicable")).toBeNull();
  });

  it("一次挂载只取一次数（回归：t 进 useCallback 依赖导致 effect 自激）", async () => {
    invokeMock.mockResolvedValue({ ...BASE, decisionDegraded: true, citations: [] });
    render(<EvidenceCitationPanel analysisId="a-3" />);
    await waitFor(() => expect(screen.queryByText("stockAnalysis.evidenceCitation.loading")).toBeNull());
    // 再等若干微任务/渲染轮次，自激 would 在此期间继续刷调用
    await new Promise((r) => setTimeout(r, 120));
    expect(invokeMock).toHaveBeenCalledTimes(1);
  });
});
