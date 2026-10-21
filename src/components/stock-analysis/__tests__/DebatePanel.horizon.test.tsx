/**
 * 辩论卡片的档位维度（v161 逐档辩论的前端呈现）。
 *
 * 修复前：四档节点 id 都被 `parseInt` 成第 1 轮，store 按 round 去重 ⇒ 卡片只剩
 * 最后完成那一档的文字。这里锁四件事：
 *   ① 四档各自成格，切档只换正文、不叠在一起；
 *   ② 整档缺席时明确说「本档没有产出」，不拿别的档顶替；
 *   ③ 历史单份记录（tier=null）仍按轮次切换（回放兼容）；
 *   ④ 默认落在第一个有数据的档（不让用户一进来就看到缺席句）。
 */
import i18n from "@/i18n";
import { useStockAnalysisStore } from "@/stores";
import { fireEvent, render, screen } from "@testing-library/react";
import { I18nextProvider } from "react-i18next";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { DebatePanel } from "../DebatePanel";

const invokeMock = vi.fn();
vi.mock("@/lib/invoke", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
  listen: vi.fn().mockResolvedValue(() => {}),
  isTauri: () => false,
}));

type Tier = "ultra_short" | "short" | "mid" | "long";

const BULL: Record<Tier, string> = {
  ultra_short: "超短多方：订单与产能同步回升，短线资金持续流入，估值仍在低位。",
  short: "短线多方：五日趋势转强，量价配合良好，板块轮动仍在延续。",
  mid: "中线多方：盈利周期见底回升，行业供需格局改善，估值具备修复空间。",
  long: "长线多方：竞争壁垒稳固，现金流持续改善，长期成长逻辑未变。",
};
const BEAR: Record<Tier, string> = {
  ultra_short: "超短空方：涨幅已透支预期，量能出现背离，回撤风险正在上升。",
  short: "短线空方：上方套牢盘密集，突破缺乏量能配合，短线不宜追高。",
  mid: "中线空方：行业扩产周期未完，价格中枢仍有下移压力，需等右侧信号。",
  long: "长线空方：需求天花板临近，资本开支回报率下行，长期估值难再扩张。",
};

function roundOf(tier: Tier, round = 1) {
  return { tier, round, bull: BULL[tier], bear: BEAR[tier] };
}

function renderPanel() {
  return render(
    <I18nextProvider i18n={i18n}>
      <DebatePanel />
    </I18nextProvider>,
  );
}

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockResolvedValue(null);
  useStockAnalysisStore.getState().reset();
});

describe("DebatePanel — 四档辩论各自成格", () => {
  it("默认显示首个有数据的档；切到短线后只显示短线正文", () => {
    useStockAnalysisStore.setState({
      debateRounds: [roundOf("ultra_short"), roundOf("short"), roundOf("mid"), roundOf("long")],
      expectedDebateRounds: 1,
    });

    renderPanel();

    // 默认档 = 第一个有数据的档（超短线）
    expect(screen.getByText(/超短多方/)).toBeTruthy();
    expect(screen.queryByText(/短线多方/)).toBeNull();

    fireEvent.click(screen.getByText(i18n.t("stockAnalysis.timeHorizonShort")));

    expect(screen.getByText(/短线多方/)).toBeTruthy();
    expect(screen.queryByText(/超短多方/)).toBeNull();
    expect(screen.queryByText(/中线多方/)).toBeNull();
  });

  it("整档缺席：切到中线时明确说明，不拿别的档顶替", () => {
    useStockAnalysisStore.setState({
      debateRounds: [roundOf("ultra_short"), roundOf("long")],
      expectedDebateRounds: 1,
    });

    renderPanel();
    fireEvent.click(screen.getByText(i18n.t("stockAnalysis.timeHorizonMid")));

    expect(
      screen.getByText(
        i18n.t("stockAnalysis.debate.tierNoOutput", { tier: i18n.t("stockAnalysis.timeHorizonMid") }),
      ),
    ).toBeTruthy();
    // 缺席档不得把上一个档的正文留在屏幕上
    expect(screen.queryByText(/超短多方/)).toBeNull();
    expect(screen.queryByText(/长线多方/)).toBeNull();
  });

  it("历史单份记录（tier=null）按轮次纵向列出，不再有轮次 tab", () => {
    useStockAnalysisStore.setState({
      debateRounds: [
        { tier: null, round: 1, bull: "历史第1轮多方：旧记录没有档位，仍按轮次展示。", bear: "历史第1轮空方。" },
        { tier: null, round: 2, bull: "历史第2轮多方：第二轮补充了新的证据与反驳。", bear: "历史第2轮空方。" },
      ],
      expectedDebateRounds: null,
    });

    renderPanel();

    // 两轮同时可见（同一档内纵向排列），而不是二选一的 tab
    expect(screen.getByText(/历史第1轮多方/)).toBeTruthy();
    expect(screen.getByText(/历史第2轮多方/)).toBeTruthy();
    expect(screen.queryByText("第1")).toBeNull();
    expect(screen.queryByText("第2")).toBeNull();
  });

  it("每档 2 轮：只有一级档位切换，轮次在档内纵向排列", () => {
    useStockAnalysisStore.setState({
      debateRounds: [
        {
          tier: "ultra_short",
          round: 1,
          bull: "超短第1轮多方：第一轮陈述，订单与产能同步回升，资金持续流入。",
          bear: "超短第1轮空方：涨幅已经透支预期，回撤风险正在上升。",
        },
        {
          tier: "ultra_short",
          round: 2,
          bull: "超短第2轮多方：第二轮反驳，量价配合仍然良好，趋势没有破坏。",
          bear: "超短第2轮空方：量能出现背离，短线不宜继续追高，等待回踩。",
        },
        {
          tier: "short",
          round: 1,
          bull: "短线第1轮多方：第一轮陈述，五日趋势转强，板块轮动仍在延续。",
          bear: "短线第1轮空方：上方套牢盘密集，突破缺乏量能配合。",
        },
        {
          tier: "short",
          round: 2,
          bull: "短线第2轮多方：第二轮反驳，突破后回踩确认，筹码结构改善。",
          bear: "短线第2轮空方：回踩失败则趋势转弱，需要严格控制仓位。",
        },
      ],
      expectedDebateRounds: 2,
    });

    renderPanel();

    // 默认超短档：两轮正文同时可见
    expect(screen.getByText(/超短第1轮多方/)).toBeTruthy();
    expect(screen.getByText(/超短第2轮多方/)).toBeTruthy();
    // 没有第二组「第1/第2」tab（旧实现会渲染成无空格的 tab 标签）
    expect(screen.queryByText("第1")).toBeNull();
    expect(screen.queryByText("第2")).toBeNull();

    fireEvent.click(screen.getByText(i18n.t("stockAnalysis.timeHorizonShort")));

    expect(screen.getByText(/短线第1轮多方/)).toBeTruthy();
    expect(screen.getByText(/短线第2轮多方/)).toBeTruthy();
    expect(screen.queryByText(/超短第1轮多方/)).toBeNull();
  });
});
