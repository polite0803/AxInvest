import i18n from "@/i18n";
import { type SerenityCandidate, useSerenityStore } from "@/stores/feature/serenityStore";
import { act, render, screen } from "@testing-library/react";
import { I18nextProvider } from "react-i18next";
import { MemoryRouter } from "react-router-dom";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { SerenityCandidateCard } from "../SerenityCandidateCard";
import { SerenityScreeningPanel } from "../SerenityScreeningPanel";

/**
 * 趋势智选**候选卡片**的档位与风控呈现（**一票一卡**：卡内分行列多档，最多四档 超短/短/中/长）。
 *
 * 反控形态（修复前必红，逐条对应）：
 *  - 卡片只有一行时间基线，**没有任何档位维度** ⇒ Q2 把产出层改成逐档两行后，
 *    面板看上去与改之前一模一样（「四周期适配」只活在 DB 与历史弹窗里）；
 *  - 时间基线兜底 `?? 20`：`Period::Mid` 的权威天数是 28，落库侧同一个矛盾已修，
 *    呈现层留着它等于继续对用户报错窗口；
 *  - 未标档（实时候选）被兜底成一个数 ⇒ 把「没有档」压成读数。
 */
vi.mock("@/lib/invoke", () => ({
  // 面板挂载即 `invoke(...).then(...)` ⇒ mock 必须返回 Promise，
  // 返回 undefined 会炸在 `.then` 上（本仓测试的同族坑，见 webview 事件测试的注释）
  invoke: vi.fn().mockResolvedValue(null),
  listen: vi.fn().mockResolvedValue(() => {}),
  isTauri: () => false,
}));

/**
 * 收尾：等挂载副作用（数据请求）落地。
 *
 * 为什么必须有：页面挂载后会异步拉数据，测试同步结束 ⇒ setState 落在测试之外 ⇒
 * React 报 "not wrapped in act"。为什么必须放在**断言之后**：这些用例断言的是
 * **初始渲染态**，提前 flush 会把组件推进"加载完成"态，断言随之失效（实测会让 3 条用例变红）。
 * 为什么不能用 afterEach：React 是在 setState 那一刻检查的，而那一刻在测试函数返回后、
 * afterEach 之前 —— 收尾钩子永远太晚。
 */
const settle = async () => {
  await act(async () => {});
};

function renderCard(candidate: SerenityCandidate) {
  return render(
    <MemoryRouter>
      <I18nextProvider i18n={i18n}>
        <SerenityCandidateCard candidate={candidate} />
      </I18nextProvider>
    </MemoryRouter>,
  );
}

const MID_ROW: SerenityCandidate = {
  stock_code: "600519",
  stock_name: "落库中期",
  serenity_score: 82,
  period: "mid",
  holding_days: 28,
  generated_at: "2026-09-30T02:00:00Z",
  price: 100,
  stopLoss: 87.3,
  targetPrice: 125.4,
  entryLow: 93.65,
  entryHigh: 106.35,
  positionPct: 4.86,
  stopSource: "vol",
  entrySource: "half_stop",
};

describe("卡片档位徽标（A）", () => {
  it("落库行按 period 出档名，走全仓唯一键族（中期 = stockAnalysis.timeHorizonMid）", () => {
    renderCard(MID_ROW);
    expect(screen.getByTestId("serenity-tier").textContent).toBe("中线");
  });

  it("实时候选没有 period ⇒ 出「未知周期」，不猜成任何一档", () => {
    renderCard({ stock_code: "600519", stock_name: "实时候选", serenity_score: 70 });
    const badge = screen.getByTestId("serenity-tier").textContent;
    expect(badge).toBe("未知周期");
    // 反控：旧三元/兜底会把它显示成某一档，或直接没有这个徽标
    expect(badge).not.toContain("中线");
  });

  it("认不出的档名原样出「未知周期」，不映射到最近的一档", () => {
    renderCard({ ...MID_ROW, period: "midterm" });
    expect(screen.getByTestId("serenity-tier").textContent).toBe("未知周期");
  });
});

describe("时间基线不再兜底 20 天（A）", () => {
  it("有 holding_days 时按该档权威天数给窗口", () => {
    renderCard(MID_ROW);
    expect(screen.getByText(/28\s*天/)).toBeTruthy();
  });

  it("无 holding_days 时出「未标档 ⇒ 无建议窗口」句，页面上不出现 20 这个读数", () => {
    const { container } = renderCard({
      stock_code: "600519",
      stock_name: "实时候选",
      serenity_score: 70,
    });
    expect(
      screen.getByText("实时候选未标档 ⇒ 无建议窗口（逐档记录见趋势智选历史）"),
    ).toBeTruthy();
    // 反控：旧形态 `?? 20` 会让文案里出现「约 20 天」
    expect(container.textContent).not.toContain("20 天");
    expect(container.textContent).not.toContain("约 20");
  });
});

describe("逐档风控与口径来源（A）", () => {
  it("落库行展示止损/目标/仓位/建仓带四个读数", () => {
    renderCard(MID_ROW);
    const text = document.body.textContent ?? "";
    expect(text).toContain("止损位 87.30");
    expect(text).toContain("目标价 125.40");
    expect(text).toContain("建议仓位 4.9%");
    // (entryHigh-entryLow)/2/price = 6.35%
    expect(text).toContain("建仓带 ±6.35%");
  });

  it("σ 不可得时两条来源各成一句，不与波动率口径混读", () => {
    renderCard({
      ...MID_ROW,
      stopSource: "fallback_pct",
      entrySource: "half_stop_fallback_pct",
    });
    const text = document.body.textContent ?? "";
    expect(text).toContain("止损口径：固定百分比（该票日线 σ 不可得，非波动率推导）");
    expect(text).toContain("建仓带取自固定止损乘数的一半 ⇒ 非波动率推导");
  });

  it("连止损距离都不可得 ⇒ 建仓带退回模板幅度，单独成句", () => {
    renderCard({ ...MID_ROW, entrySource: "fallback_range" });
    expect(document.body.textContent).toContain(
      "止损距离不可得 ⇒ 建仓带退回模板固定幅度",
    );
  });

  it("波动率口径的正常路径不弹退化句（退化声明不得无条件挂）", () => {
    renderCard(MID_ROW);
    expect(document.body.textContent).not.toContain("非波动率推导");
  });
});

describe("面板一票一卡（逐档折叠，最多四档）", () => {
  beforeEach(() => {
    act(() => {
      useSerenityStore.setState({ candidates: [], running: false, steps: [] });
    });
  });

  function renderPanel() {
    render(
      <MemoryRouter>
        <I18nextProvider i18n={i18n}>
          <SerenityScreeningPanel />
        </I18nextProvider>
      </MemoryRouter>,
    );
  }

  it("同票两档 ⇒ 折叠成一张卡、卡内两行档位（中线/长线），标题按票计数", async () => {
    act(() => {
      useSerenityStore.setState({
        candidates: [
          MID_ROW,
          { ...MID_ROW, stock_name: "落库长期", period: "long", holding_days: 90 },
        ],
      });
    });
    renderPanel();
    expect(screen.getAllByTestId("serenity-tier").map((el) => el.textContent)).toEqual([
      "中线",
      "长线",
    ]);
    // 一票一卡 ⇒ 卡头（代表候选）只渲染一次；第二档的名字不出现在卡头
    expect(screen.getAllByText("落库中期")).toHaveLength(1);
    expect(screen.queryByText("落库长期")).toBeNull();
    expect(screen.getByText(/候选标的\s*\(1\s*只\)/)).toBeTruthy();
    await settle();
  });

  it("四档同票 ⇒ 一张卡内四行，按 超短→短→中→长 排列", async () => {
    act(() => {
      useSerenityStore.setState({
        candidates: [
          { ...MID_ROW, period: "mid", holding_days: 28 },
          { ...MID_ROW, period: "long", holding_days: 90 },
          { ...MID_ROW, period: "ultra_short", holding_days: 5 },
          { ...MID_ROW, period: "short", holding_days: 10 },
        ],
      });
    });
    renderPanel();
    expect(screen.getAllByTestId("serenity-tier").map((el) => el.textContent)).toEqual([
      "超短线",
      "短线",
      "中线",
      "长线",
    ]);
    expect(screen.getByText(/候选标的\s*\(1\s*只\)/)).toBeTruthy();
    await settle();
  });

  it("两票 ⇒ 两张卡，标题按票数计 2 只", async () => {
    act(() => {
      useSerenityStore.setState({
        candidates: [
          MID_ROW,
          { ...MID_ROW, stock_code: "300750", stock_name: "另一只", period: "mid" },
        ],
      });
    });
    renderPanel();
    expect(screen.getAllByTestId("serenity-tier")).toHaveLength(2);
    expect(screen.getByText(/候选标的\s*\(2\s*只\)/)).toBeTruthy();
    await settle();
  });
});
