import { describe, expect, it } from "vitest";
import { analystStoreKeyOf, dqAbbrOf, reportQualitySeverity, resolveAnalystDiagnosis } from "../dataQualityDiagnosis";

// 2026-09-21 新增：`report_quality`（0-100）→ 展示严重度的**单一来源**。
//
// 为什么值得单测：该映射原先只写在 `AnalystDataQualityModal` 的行内三目表达式里，
// 同日 `DecisionBanner` 的逐节点表格也要用同一档位 ⇒ 抽到本模块。
// 一旦有人「顺手」在某个组件里改写阈值，就会出现**同一个数值在两处显示不同颜色**
// （本项目当天正在清理的「同语义多份判据」形态）。以下边界正是两处共用的契约。
describe("reportQualitySeverity —— 单节点报告质量分的档位（单一来源）", () => {
  it("undefined ⇒ null（无值，**不是** good）", () => {
    // 旧版快照没有 report_quality 字段。返回 null 而非 "good"，
    // 否则调用方会把「不知道」渲染成绿色对勾。
    expect(reportQualitySeverity(undefined)).toBeNull();
  });

  it("0 是**有值**，属于最低档 issue（不能与 undefined 混为一谈）", () => {
    // 0 有明确语义：该节点未注入报告（data-quality.rhai 里 report_quality("", conf) 的取值）。
    expect(reportQualitySeverity(0)).toBe("issue");
  });

  it("80 是 good 的下界（含 80）", () => {
    expect(reportQualitySeverity(80)).toBe("good");
    expect(reportQualitySeverity(100)).toBe("good");
  });

  it("79.99 ⇒ warning（不被四舍五入进 good）", () => {
    expect(reportQualitySeverity(79.99)).toBe("warning");
  });

  it("50 是 warning 的下界（含 50）", () => {
    expect(reportQualitySeverity(50)).toBe("warning");
  });

  it("49.99 ⇒ issue", () => {
    expect(reportQualitySeverity(49.99)).toBe("issue");
  });
});

/**
 * 键形态归一（2026-10-25 本批第一步）。
 *
 * 实测断点：`EXPERT_ID_TO_DQ_ABBR` 的键域是**节点 id**（`a-market-analyst`），
 * 而 store 落 `analystReports` 的键是**去 `a-` 前缀**的 base（`market-analyst`），
 * 逐档实例还再带 `--<tier>` 后缀 ⇒ 旧实现直接精确查表时 10 个分析师**全部落空**，
 * 只靠「比对后端中文角色名与界面译文」那条末级兜底救回 4 个。查表恒落空却被兜底
 * 掩盖成「看起来正常」，正是本仓反复登记的失效形状，故在此钉住四种键形态。
 */
describe("dqAbbrOf / resolveAnalystDiagnosis —— 四种键形态都认", () => {
  const mk = (conf: number, name = "技术面分析师") => ({
    name,
    expected_data: "K线",
    confidence: conf,
    status: "normal" as const,
    gap_reason: "",
    inconsistent_reason: "",
  });
  const report = {
    grade: "B",
    score: 70,
    diagnostics: { mk: mk(50), val: mk(51, "价值投资分析师") },
    diagnostics_by_tier: { mk: { ultra_short: mk(20), mid: mk(80) } },
    missing_analysts: [],
    low_confidence_analysts: [],
  } as unknown as import("@/types").DataQualityReport;

  it("节点 id / store 键 / 带档实例（两种前缀形态）都归一到同一缩写", () => {
    expect(dqAbbrOf("a-market-analyst")).toBe("mk");
    expect(dqAbbrOf("market-analyst")).toBe("mk");
    expect(dqAbbrOf("a-market-analyst--mid")).toBe("mk");
    expect(dqAbbrOf("market-analyst--ultra_short")).toBe("mk");
    // 无 a- 前缀的历史槽位（value-investor）同样命中
    expect(dqAbbrOf("value-investor")).toBe("val");
  });

  it("非分析师节点 ⇒ undefined（不猜缩写）", () => {
    expect(dqAbbrOf("bull-r1")).toBeUndefined();
    expect(dqAbbrOf("data-quality")).toBeUndefined();
  });

  it("指明代档 ⇒ 取该档那一行；不指代 ⇒ 取代表实例", () => {
    expect(resolveAnalystDiagnosis("market-analyst", report, undefined, "ultra_short")?.confidence).toBe(20);
    expect(resolveAnalystDiagnosis("market-analyst", report, undefined, "mid")?.confidence).toBe(80);
    // 超短卡点开弹窗却显示中线那行的置信度 —— 加 tier 参数就是为了不再出现这种同屏两数
    expect(resolveAnalystDiagnosis("market-analyst", report)?.confidence).toBe(50);
  });

  it("该档没有逐档行 ⇒ 回落代表实例，不返回 null（旧快照 / 快速链仍可读）", () => {
    expect(resolveAnalystDiagnosis("market-analyst", report, undefined, "long")?.confidence).toBe(50);
  });

  it("译文兜底只在缩写查不到时生效：id 命中时不受 name 影响", () => {
    // 表命中走 mk（50）；即便 name 传成别的角色名也不串槽
    expect(resolveAnalystDiagnosis("market-analyst", report, "研报分析师")?.confidence).toBe(50);
    // 未知节点 id ⇒ 末级兜底按后端中文角色名比对才命中
    expect(resolveAnalystDiagnosis("a-unknown-node", report, "技术面分析师")?.confidence).toBe(50);
  });

  it("analystStoreKeyOf：剥 a- 前缀只在这里做一次", () => {
    expect(analystStoreKeyOf("a-market-analyst")).toBe("market-analyst");
    expect(analystStoreKeyOf("value-investor")).toBe("value-investor");
  });
});
