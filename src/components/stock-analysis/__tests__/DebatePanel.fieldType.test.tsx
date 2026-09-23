/**
 * 回归：辩论 tab 整页崩溃（ErrorBoundary「页面错误」）
 *
 * 真实成因（2026-09-22 DB 实证，分析 300642 / id 8d8644a3 的 bull-r3）：
 * 辩手节点 content 里的 `final_position` 实际是**对象**
 * `{portfolio_mgr_input, reason, stance:"弱看多"}`，而 R3View 直接把它交给
 * `r3PosLabel()` → `key.toLowerCase()` ⇒ `TypeError: key.toLowerCase is not a function`
 * ⇒ 整页「页面错误」。
 *
 * 分派层的 `Array.isArray` 守卫只管顶层：`isR3` 因 r2_cross_examination_response 是数组
 * 而成立，于是进入 R3View，那里对 final_position 的类型假设无人保证。
 *
 * 本组测试用**真实 payload 形态**锁住四件事：
 *   1. 对象型 final_position 不再抛错（修复前必崩）
 *   2. 对象里的 stance 被提取出来展示（否则卡片只有空标签）
 *   3. strengthened_arguments 的 argument/evidence 别名被映射（不崩但全空的另一半）
 *   4. questions 为字符串时 R2View 不崩（该字段无分派守卫，最易踩）
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

/** 真实 bull-r3 payload（取自 DB 300642 / 8d8644a3，字段名与值均未改动） */
const REAL_BULL_R3 = JSON.stringify({
  data_gaps: ["缺少主力资金连续净流入的确证数据"],
  final_position: {
    portfolio_mgr_input: "Strength Score 38, Confidence 42. 建议策略由'积极配置'转为'防御性持有/等待左侧买点'。",
    reason: "核心论点在质询后显著削弱，仅保留财务底线支撑，不足以驱动激进买入。",
    stance: "弱看多",
  },
  r2_cross_examination_response: [
    { r2_question_ref: "估值锚定是否可靠", response: "承认估值分位偏高但认为成长可消化", verdict: "部分接受" },
  ],
  strengthened_arguments: [
    { argument: "财务底线支撑", evidence: "经营现金流连续三季为正", status: "保留" },
  ],
});

/** bull-r2：questions 是字符串（模拟 LLM 未按数组输出） */
const STRING_QUESTIONS_R2 = JSON.stringify({
  cross_examination: [
    {
      target_claim_ref: "成长逻辑",
      weakness_type: "证据不足",
      questions: "增长假设是否有订单支撑？；毛利率能否维持？",
    },
  ],
  summary_for_convergence: "质询聚焦于增长可持续性",
});

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

describe("DebatePanel — LLM 输出字段类型不受控时的健壮性", () => {
  it("final_position 是对象时不崩，并提取出 stance 展示（修复前 ⇒ TypeError 整页崩）", () => {
    useStockAnalysisStore.setState({ debateRounds: [{ round: 3, bull: REAL_BULL_R3, bear: "" }] });

    // 修复前：R3View:555 r3PosLabel(对象) → key.toLowerCase() 抛 TypeError ⇒ render 直接抛
    expect(() => renderPanel()).not.toThrow();

    // 对象里的 stance 必须被**提取**出来（而不是把整个对象 stringify 后塞进 DOM）
    expect(screen.getAllByText(/弱看多/).length).toBeGreaterThan(0);
    // 关键区分力断言：不得把对象序列化形态泄漏到界面
    expect(screen.queryByText(/portfolio_mgr_input/)).toBeNull();
  });

  it("strengthened_arguments 用 argument/evidence 也能显示（别名映射，防「不崩但全空」）", () => {
    useStockAnalysisStore.setState({ debateRounds: [{ round: 3, bull: REAL_BULL_R3, bear: "" }] });

    renderPanel();

    expect(screen.getByText(/财务底线支撑/)).toBeTruthy();
    expect(screen.getByText(/经营现金流连续三季为正/)).toBeTruthy();
  });

  it("r2 质询的 questions 是字符串（而非数组）时 R2View 不崩", () => {
    useStockAnalysisStore.setState({ debateRounds: [{ round: 2, bull: STRING_QUESTIONS_R2, bear: "" }] });

    // 修复前：R2View:487 ce.questions.map ⇒ questions.map is not a function
    expect(() => renderPanel()).not.toThrow();

    // 字符串按「；」切分后仍完整保留信息
    expect(screen.getByText(/增长假设是否有订单支撑/)).toBeTruthy();
    expect(screen.getByText(/毛利率能否维持/)).toBeTruthy();
  });

  it("正常字符串 final_position 走原有映射（防修复误伤既有路径）", () => {
    const normalR3 = JSON.stringify({
      final_position: "weak_bull",
      strengthened_arguments: [{ claim_ref: "财务底线", additional_evidence: "现金流为正", final_strength: 60 }],
    });
    useStockAnalysisStore.setState({ debateRounds: [{ round: 3, bull: normalR3, bear: "" }] });

    renderPanel();

    // weak_bull 命中 r3PosLabel 的映射表 ⇒ 走 i18n 文案（zh-CN 回退）
    expect(screen.getAllByText(new RegExp(i18n.t("stockAnalysis.debate.weakBullish"))).length).toBeGreaterThan(0);
    expect(screen.getByText(/财务底线/)).toBeTruthy();
  });
});
