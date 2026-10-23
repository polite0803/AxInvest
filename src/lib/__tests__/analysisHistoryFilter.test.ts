import { readFileSync } from "node:fs";
import path from "node:path";
import { describe, expect, it } from "vitest";
import {
  filterAnalysisHistory,
  getActionTKey,
  horizonOfRecord,
  mergeAnalysisHistoryPages,
  provenanceOfRecord,
  StockAction,
  UNKNOWN_HORIZON,
} from "../stock-analysis-utils";

/**
 * 历史分析弹窗「全量加载 + 筛选」的判据层。
 *
 * 钉住三件复发代价最高的事：
 *   ① 筛选判据必须与**呈现层同一查表**（`HORIZON_T_SUFFIX`）—— 现网有 34 行
 *      `decision_time_horizon` 存的是模型吐出的中文「短期」，界面上显示为「未知周期」；
 *      若筛选按原始值等值比较，选「未知周期」漏掉这 34 行、选「短线」又命中它们，
 *      就是「筛出来的和看见的不是一回事」。
 *   ② 三个维度各自独立取交集（漏一个维度就会把不该看的行放进来）。
 *   ③ 分页合并按 id 去重 —— 否则并发落库造成的重叠页会被显示成「记录变多了」。
 */

const rec = (
  id: string,
  stockCode: string,
  stockName: string,
  extra: Record<string, unknown> = {},
) => ({ id, stockCode, stockName, status: "completed", createdAt: 1, ...extra });

describe("filterAnalysisHistory · 关键词", () => {
  const rows = [
    rec("a", "600519", "贵州茅台"),
    rec("b", "000001", "平安银行"),
    rec("c", "300750", "宁德时代"),
  ];

  it("命中代码或名称，大小写不敏感且首尾空白不参与", () => {
    expect(filterAnalysisHistory(rows, { keyword: " 600519 " }).map((r) => r.id)).toEqual(["a"]);
    expect(filterAnalysisHistory(rows, { keyword: "时代" }).map((r) => r.id)).toEqual(["c"]);
  });

  it("空关键词 = 该维度不筛（全量原序返回，不得顺手裁掉任何一行）", () => {
    expect(filterAnalysisHistory(rows, { keyword: "" })).toHaveLength(3);
    expect(filterAnalysisHistory(rows, { keyword: "   " })).toHaveLength(3);
  });
});

describe("filterAnalysisHistory · 状态", () => {
  const rows = [
    rec("a", "600519", "贵州茅台", { status: "completed" }),
    rec("b", "600519", "贵州茅台", { status: "interrupted" }),
    rec("c", "600519", "贵州茅台", { status: "running" }),
  ];

  it("按 status 等值裁剪，undefined 时不筛", () => {
    expect(filterAnalysisHistory(rows, { keyword: "", status: "interrupted" }).map((r) => r.id))
      .toEqual(["b"]);
    expect(filterAnalysisHistory(rows, { keyword: "" })).toHaveLength(3);
  });

  it("库里出现清单外的态时不得被任何已选态顶替（选 completed 不放水）", () => {
    const odd = [...rows, rec("d", "600519", "贵州茅台", { status: "weird_state" })];
    expect(filterAnalysisHistory(odd, { keyword: "", status: "completed" }).map((r) => r.id))
      .toEqual(["a"]);
  });
});

describe("filterAnalysisHistory · 档位与呈现同判据", () => {
  const rows = [
    rec("a", "600519", "贵州茅台", { decisionTimeHorizon: "ultra_short" }),
    rec("b", "600519", "贵州茅台", { decisionTimeHorizon: "short" }),
    rec("c", "600519", "贵州茅台", { decisionTimeHorizon: "mid" }),
    rec("d", "600519", "贵州茅台", { decisionTimeHorizon: "long" }),
    rec("e", "600519", "贵州茅台", { decisionTimeHorizon: null }),
    rec("f", "600519", "贵州茅台"),
    // 现网真实脏值：模型把中文档位名塞进该列（界面上显示为「未知周期」）
    rec("g", "600519", "贵州茅台", { decisionTimeHorizon: "短期" }),
  ];

  it("四档各归各，null / 缺列 / 中文脏值一律归「未知周期」", () => {
    const idsByTier = Object.fromEntries(
      (["ultra_short", "short", "mid", "long"] as const).map((tier) => [
        tier,
        filterAnalysisHistory(rows, { keyword: "", horizon: tier }).map((r) => r.id),
      ]),
    );
    // 一次断四格：vitest 的 diff 会直接点出是哪个档位串了行
    expect(idsByTier).toEqual({ ultra_short: ["a"], short: ["b"], mid: ["c"], long: ["d"] });
    expect(filterAnalysisHistory(rows, { keyword: "", horizon: UNKNOWN_HORIZON }).map((r) => r.id))
      .toEqual(["e", "f", "g"]);
  });

  it("负控：选「短线」不得把中文「短期」那行算进来（同判据才会一致漏掉）", () => {
    const hit = filterAnalysisHistory(rows, { keyword: "", horizon: "short" }).map((r) => r.id);
    expect(hit).toEqual(["b"]);
    expect(hit).not.toContain("g");
  });

  it("horizonOfRecord 与查表同源：不在 HORIZON_T_SUFFIX 里的值不得被猜成某档", () => {
    expect(horizonOfRecord("mid")).toBe("mid");
    expect(horizonOfRecord(null)).toBe(UNKNOWN_HORIZON);
    expect(horizonOfRecord(undefined)).toBe(UNKNOWN_HORIZON);
    expect(horizonOfRecord("")).toBe(UNKNOWN_HORIZON);
    expect(horizonOfRecord("MID")).toBe(UNKNOWN_HORIZON);
  });
});

describe("filterAnalysisHistory · 维度取交集", () => {
  const rows = [
    rec("a", "600519", "贵州茅台", { status: "completed", decisionTimeHorizon: "mid" }),
    rec("b", "600519", "贵州茅台", { status: "interrupted", decisionTimeHorizon: "mid" }),
    rec("c", "600519", "贵州茅台", { status: "completed", decisionTimeHorizon: "long" }),
    rec("d", "000001", "平安银行", { status: "completed", decisionTimeHorizon: "mid" }),
  ];

  it("关键词 + 状态 + 档位三条同时生效", () => {
    expect(
      filterAnalysisHistory(rows, {
        keyword: "600519",
        status: "completed",
        horizon: "mid",
      }).map((r) => r.id),
    ).toEqual(["a"]);
  });

  it("只开档位时，状态不同的同档记录必须一起出来（不得顺手加状态门）", () => {
    expect(filterAnalysisHistory(rows, { keyword: "", horizon: "mid" }).map((r) => r.id))
      .toEqual(["a", "b", "d"]);
  });
});

describe("filterAnalysisHistory · 结论档与 Tag 同判据", () => {
  const rows = [
    rec("a", "600519", "贵州茅台", { decisionAction: "买入" }),
    rec("b", "600519", "贵州茅台", { decisionAction: "HOLD" }),
    rec("c", "600519", "贵州茅台", { decisionAction: "数据缺失" }),
    // 未识别的原始值：Tag 走 getActionTKey 的 default ⇒ 显示「不确定」，筛选必须同格
    rec("d", "600519", "贵州茅台", { decisionAction: "满仓干" }),
    // 有 decisionJson 但里面没有 action ⇒ 既有口径同样归「不确定」（不是「数据缺失」）
    rec("e", "600519", "贵州茅台", { decisionJson: JSON.stringify({ confidence: 40 }) }),
    // 还没有结论：既无直返字段也无可解析对象（列表里只有 running 行会是这形态）
    rec("f", "600519", "贵州茅台"),
    rec("g", "600519", "贵州茅台", { decisionJson: "null" }),
  ];
  const ids = (action: string) => filterAnalysisHistory(rows, { keyword: "", action }).map((r) => r.id);

  it("中文与英文原始值归同一档（后端实发中文，DTO 亦有英文形态）", () => {
    expect(ids(StockAction.BUY)).toEqual(["a"]);
    expect(ids(StockAction.HOLD)).toEqual(["b"]);
    expect(ids(StockAction.UNAVAILABLE)).toEqual(["c"]);
  });

  it("未识别值与「json 里没 action」都归「不确定」", () => {
    expect(ids(StockAction.UNCERTAIN)).toEqual(["d", "e"]);
  });

  // 裁定后不再有自造的「无结论」档：后端对空 decision_action 下发 UNAVAILABLE 哨兵，
  // 「还没有结论」只剩 running 行（由状态维度表达）⇒ 它不属于任何一档。
  it("没有任何结论的行不属于任何一档", () => {
    for (const a of Object.values(StockAction)) {
      const hit = ids(a);
      expect(hit).not.toContain("f");
      expect(hit).not.toContain("g");
    }
  });

  it("负控：选「买入」不得把「没结论」或「不确定」的行放进来", () => {
    const hit = ids(StockAction.BUY);
    expect(hit).not.toContain("f");
    expect(hit).not.toContain("g");
    expect(hit).not.toContain("d");
  });

  it("结论维度与状态/档位维度取交集", () => {
    const mixed = [
      rec("x", "600519", "贵州茅台", { decisionAction: "买入", status: "completed", decisionTimeHorizon: "mid" }),
      rec("y", "600519", "贵州茅台", { decisionAction: "买入", status: "interrupted", decisionTimeHorizon: "mid" }),
      rec("z", "600519", "贵州茅台", { decisionAction: "买入", status: "completed", decisionTimeHorizon: "long" }),
    ];
    expect(
      filterAnalysisHistory(mixed, {
        keyword: "",
        action: StockAction.BUY,
        status: "completed",
        horizon: "mid",
      }).map((r) => r.id),
    ).toEqual(["x"]);
  });

  it("八个结论选项的展示键互不相同且都在 zh-CN 里（动态键逃过 check_i18n 的静态扫描）", () => {
    const zh = JSON.parse(
      readFileSync(path.resolve(import.meta.dirname, "..", "..", "i18n", "locales", "zh-CN.json"), "utf8"),
    ) as { stockAnalysis: Record<string, string> };
    const keys = Object.values(StockAction).map((a) => getActionTKey(a));
    expect(new Set(keys).size).toBe(8);
    for (const k of keys) {
      const leaf = k.split(".").pop() as string;
      expect(typeof zh.stockAnalysis[leaf], `zh-CN 缺 ${k}`).toBe("string");
      expect(zh.stockAnalysis[leaf].length, `zh-CN ${k} 是空串`).toBeGreaterThan(0);
    }
  });
});

describe("mergeAnalysisHistoryPages", () => {
  const rows = [rec("a", "1", "A"), rec("b", "2", "B"), rec("c", "3", "C")];

  it("按 id 去重且保持「先到在前」= 后端 created_at 倒序", () => {
    expect(mergeAnalysisHistoryPages([rows[0]], [rows[1], rows[0], rows[2]]).map((r) => r.id))
      .toEqual(["a", "b", "c"]);
  });

  it("重复页（并发落库把窗口推回去）不得让条数虚增", () => {
    const once = mergeAnalysisHistoryPages([], rows);
    expect(mergeAnalysisHistoryPages(once, rows).map((r) => r.id)).toEqual(["a", "b", "c"]);
  });

  it("空页与空集合都不吞已有数据", () => {
    expect(mergeAnalysisHistoryPages(rows, [])).toHaveLength(3);
    expect(mergeAnalysisHistoryPages([], [])).toHaveLength(0);
  });

  it("不改动入参数组（React state 更新要的是新引用）", () => {
    const loaded = [rows[0]];
    const merged = mergeAnalysisHistoryPages(loaded, [rows[1]]);
    expect(loaded).toHaveLength(1);
    expect(merged).not.toBe(loaded);
  });
});

describe("provenanceOfRecord · 记录来历（live / as-of 回放）", () => {
  it("写侧两态各自归位（判据只认 analysis_kind，不认 as_of_date 是否为空）", () => {
    expect(provenanceOfRecord({ analysisKind: "live" })).toBe("live");
    expect(provenanceOfRecord({ analysisKind: "replay" })).toBe("replay");
    // 现网实测：283 行的 as_of_date 全部非空（含 197 条 live）⇒ 拿它判来历会把每条都判成回放
    expect(provenanceOfRecord({ analysisKind: "live", asOfDate: "2026-10-01" } as never)).toBe("live");
  });

  it("大小写与空格不参与判定（值域由写侧产生，展示端不得因措辞漂出两态）", () => {
    expect(provenanceOfRecord({ analysisKind: " Replay " })).toBe("replay");
    expect(provenanceOfRecord({ analysisKind: "LIVE" })).toBe("live");
  });

  it("缺失与第三种值 ⇒ unknown，不得默认成 live 或 replay", () => {
    expect(provenanceOfRecord({})).toBe("unknown");
    expect(provenanceOfRecord({ analysisKind: null })).toBe("unknown");
    expect(provenanceOfRecord({ analysisKind: "" })).toBe("unknown");
    expect(provenanceOfRecord({ analysisKind: "backfill" })).toBe("unknown");
  });

  it("弹窗确实把这两列声明进记录类型并接上来历判据（只改后端下发、组件仍读旧形状＝半改状态）", () => {
    const src = readFileSync(
      path.resolve(import.meta.dirname, "..", "..", "components", "stock-analysis", "AnalysisHistoryButton.tsx"),
      "utf8",
    );
    expect(src).toContain("analysisKind?: string | null;");
    expect(src).toContain("asOfDate?: string | null;");
    expect(src).toContain("provenanceOfRecord(r)");
  });
});
