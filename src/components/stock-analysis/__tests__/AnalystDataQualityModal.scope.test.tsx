// 数据质量面板的**作用域**回归门禁（2026-09-21）。
//
// 背景（用户报「所有分析师节点的数据质量监控都是这个结论，这是造假吗」）：
//   面板顶部的 score / grade / good / degraded / gap 取自 data-quality 节点的**全局**输出
//   对象，与 expertId 无关 ⇒ 10 张分析师卡片打开后显示同一组数字。这是**设计使然**
//   （全局对象里没有 per-node 等级），但当时面板没有任何作用域标注，用户必然误读为
//   「这个分析师的分数」，并据此怀疑数据造假。
//
// 本测试锁住修复后的三层语义，防止回归：
//   ① 顶部数字**必须**在所有分析师卡片间保持一致（它就是全局值，不许悄悄改成 per-node——
//      那会再造「两套同名等级」，是 2026-09-14 才修掉的坑）；
//   ② 中文表格里的「本节点报告质量」**必须**随 expertId 变化（本次新增的 per-node 量）；
//   ③ 旧快照（无 report_quality 字段）必须降级为「无此字段」而不是显示 0/NaN。
//
// i18n 刻意 mock 成「key + 插值参数」拼接：本测试断言的是**组件把哪个值交给了 i18n**，
// 不是译文本身（译文由 `scripts/check_i18n.py` 与 11 语言齐备性检查负责）。
import { cleanup, render } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { AnalystDataQualityModal } from "../AnalystDataQualityModal";

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, opts?: Record<string, unknown>) =>
      opts ? `${key}${Object.entries(opts).map(([k, v]) => `|${k}=${v}`).join("")}` : key,
  }),
  initReactI18next: { type: "3rdParty", init: () => {} },
}));

// 组件在打开时会 invoke("save_node_feedback") 上报自我进化数据，测试中无需真实后端。
vi.mock("@/lib/invoke", () => ({ invoke: vi.fn().mockResolvedValue(undefined) }));

const storeState = { dataQualitySummary: "" };
vi.mock("@/stores", () => ({
  useStockAnalysisStore: (sel: (s: typeof storeState) => unknown) => sel(storeState),
}));

/** 构造一个 diagnostics 条目；`rq === undefined` 表示旧版快照（无该字段）。 */
function item(name: string, confidence: number, rq: number | undefined) {
  return {
    name,
    expected_data: "x",
    confidence,
    status: confidence >= 50 ? "normal" : "low",
    gap_reason: "",
    placeholder_hits: 0,
    placeholder_occurrences: 0,
    ...(rq === undefined ? {} : { report_quality: rq }),
  };
}

function summary(withRq: boolean): string {
  return JSON.stringify({
    grade: "B",
    score: 79.2,
    report_quality_score: 61.5,
    tool_credibility_score: 64.5,
    factor_completeness_pct: 100,
    good_count: 1,
    degraded_count: 1,
    gap_count: 0,
    total_analysts: 10,
    diagnostics: {
      mk: item("技术面分析师", 72, withRq ? 88 : undefined),
      hm: item("资金面分析师", 55, withRq ? 42.5 : undefined),
    },
  });
}

/**
 * 渲染并取回 body 文本（antd Modal 走 portal，内容挂到 document.body）。
 *
 * ⚠ 渲染**前**必须 cleanup：antd Modal 的 portal 挂 document.body，
 *   同一个用例里连续 render 两次会让两个弹窗的文本同时留在 body 里，
 *   `not.toContain(...)` 这类反向断言就会假红（首次实测即踩：技术面的 score=88
 *   出现在资金面的断言文本里，看起来像「取错诊断条目」，实为上一个弹窗未清理）。
 */
function bodyTextOf(expertId: string, name: string): string {
  cleanup();
  render(<AnalystDataQualityModal name={name} expertId={expertId} open onClose={() => {}} />);
  return document.body.textContent ?? "";
}

describe("AnalystDataQualityModal — 全局值 vs 本节点值（2026-09-21）", () => {
  beforeEach(() => {
    storeState.dataQualitySummary = summary(true);
  });

  it("顶部四数在所有分析师卡片间保持一致（它是全局聚合值，不许随 expertId 变）", () => {
    const market = bodyTextOf("a-market-analyst", "技术面分析师");
    const hotMoney = bodyTextOf("a-hot-money", "资金面分析师");

    // count=10 = report.total_analysts；两个卡片都必须渲染同一个值
    for (const [label, text] of [["技术面", market], ["资金面", hotMoney]] as const) {
      expect(text, `${label}卡片缺少全局分析师计数`).toContain("dqAnalystCount|count=10");
      // 三维分解里的全局均值也必须相同
      expect(text, `${label}卡片的三维分解应显示全局 report_quality_score`).toContain("61.5");
    }
  });

  it("有作用域标注文案（本次修复的核心：明确告诉用户上面是全局值）", () => {
    const text = bodyTextOf("a-hot-money", "资金面分析师");
    expect(text).toContain("dqGlobalScopeTitle|count=10");
    expect(text).toContain("dqGlobalScopeNote");
  });

  it("「本节点报告质量」随 expertId 变化（per-node 量确实接上了）", () => {
    const market = bodyTextOf("a-market-analyst", "技术面分析师");
    const hotMoney = bodyTextOf("a-hot-money", "资金面分析师");

    expect(market).toContain("dqFieldNodeReportQuality");
    expect(market).toContain("dqNodeReportQualityValue|score=88");
    expect(hotMoney).toContain("dqNodeReportQualityValue|score=42.5");
    // 反向：技术面的 88 不得出现在资金面卡片里（否则说明取错了诊断条目）
    expect(hotMoney).not.toContain("dqNodeReportQualityValue|score=88");
  });

  it("旧快照（无 report_quality）降级为「无此字段」，不显示 0 或 NaN", () => {
    storeState.dataQualitySummary = summary(false);
    const text = bodyTextOf("a-hot-money", "资金面分析师");
    expect(text).toContain("dqNodeReportQualityUnavailable");
    expect(text).not.toContain("dqNodeReportQualityValue");
    expect(text).not.toContain("NaN");
  });

  // 2026-09-21（本轮追加）：第 4 态 "unknown" 的行为锁。
  //
  // 此前「无值」被并入 `good` ⇒ 绿色对勾旁边写着「旧版快照无此字段」，
  // 等于**把「不知道」渲染成「好」**。现改为灰点 `MinusCircleFilled`。
  // 全表只有「本节点报告质量」一行可能取到 unknown，故用 minus-circle 计数即可锁定。
  it("旧快照那一行是灰点（unknown），不是绿色对勾 —— 「不知道」≠「好」", () => {
    storeState.dataQualitySummary = summary(false);
    bodyTextOf("a-hot-money", "资金面分析师");
    expect(document.body.querySelectorAll(".anticon-minus-circle").length).toBe(1);
  });

  it("有值时不得出现灰点（unknown 只属于「缺字段」，不能扩散成通用兜底）", () => {
    bodyTextOf("a-hot-money", "资金面分析师");
    expect(document.body.querySelectorAll(".anticon-minus-circle").length).toBe(0);
  });
});
