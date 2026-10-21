/**
 * 回归：辩论「失败轮次」判定的分母来源（2026-10-12）。
 *
 * 修复前 `DebatePanel.tsx` 写死 `EXPECTED_ROUNDS = [1, 2, 3]`、`AnalysisProgress.tsx` 写死
 * `TOTAL_DEBATE_ROUNDS = 3`。轮数是**建图参数**（后端按模板变量 `debate_rounds` 展开 N 对
 * `bull-rN`/`bear-rN` 节点），于是：
 *   - 用户设 2 轮 ⇒ 面板报「第 3 轮辩论失败」（把「本来没有」说成「跑坏了」）；
 *   - 用户设 4 轮 ⇒ 第 4 轮真失败时面板**报不出来**（期望集只到 3）；
 *   - 读不到配置 ⇒ 同样在瞎报。
 * 现在分母取自 `store.expectedDebateRounds`（`fetchExpectedDebateRounds()` 读模板变量，
 * 后端建图时与该变量强制同源）。`null` = 未知 ⇒ 不宣告任何失败。
 *
 * 四条断言各锁一个方向，其中 ②④ 是「修复前必红」的坏样本。
 */
import i18n from "@/i18n";
import { useStockAnalysisStore } from "@/stores";
import { render, screen } from "@testing-library/react";
import { I18nextProvider } from "react-i18next";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { DebatePanel } from "../DebatePanel";

const invokeMock = vi.fn();
vi.mock("@/lib/invoke", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
  listen: vi.fn().mockResolvedValue(() => {}),
  isTauri: () => false,
}));

/** 造 N 轮辩论产物（正文只要非空，判定只看 round 集合）。 */
function rounds(n: number) {
  return Array.from({ length: n }, (_, i) => ({
    // 历史单份记录：分档之前的夹具保持原形状，验证回放兼容
    tier: null,
    round: i + 1,
    bull: JSON.stringify({ core_arguments: [{ claim: `多方论点 ${i + 1}` }], stance: "bullish" }),
    bear: JSON.stringify({ core_arguments: [{ claim: `空方论点 ${i + 1}` }], stance: "bearish" }),
  }));
}

function renderPanel() {
  return render(
    <I18nextProvider i18n={i18n}>
      <DebatePanel />
    </I18nextProvider>,
  );
}

/** 失败告警是否出现（按 i18n 文案本身判，不依赖 DOM 结构）。 */
function failedBannerText(rounds: string) {
  return i18n.t("stockAnalysis.debateRoundsFailed", { rounds });
}

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockResolvedValue(null);
  useStockAnalysisStore.getState().reset();
});

describe("DebatePanel — 失败轮次判定的分母必须来自配置", () => {
  it("① 期望 3 轮、实跑 2 轮 ⇒ 报第 3 轮失败", () => {
    useStockAnalysisStore.setState({ debateRounds: rounds(2), expectedDebateRounds: 3 });

    renderPanel();

    expect(screen.getByText(failedBannerText("3"))).toBeTruthy();
  });

  it("② 期望 2 轮、实跑 2 轮 ⇒ 不得报任何失败（修复前必红：写死的 [1,2,3] 会报第 3 轮）", () => {
    useStockAnalysisStore.setState({ debateRounds: rounds(2), expectedDebateRounds: 2 });

    renderPanel();

    expect(screen.queryByText(/轮辩论失败/)).toBeNull();
  });

  it("③ 分母未知（null）⇒ 不宣告失败，「不知道」不得伪装成「跑坏了」", () => {
    useStockAnalysisStore.setState({ debateRounds: rounds(2), expectedDebateRounds: null });

    renderPanel();

    expect(screen.queryByText(/轮辩论失败/)).toBeNull();
  });

  it("④ 期望 4 轮、实跑 3 轮 ⇒ 报第 4 轮失败（修复前必红：期望集只到 3，这条失败报不出来）", () => {
    useStockAnalysisStore.setState({ debateRounds: rounds(3), expectedDebateRounds: 4 });

    renderPanel();

    expect(screen.getByText(failedBannerText("4"))).toBeTruthy();
  });
});
