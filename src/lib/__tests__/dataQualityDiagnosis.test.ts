import { describe, expect, it } from "vitest";
import { reportQualitySeverity } from "../dataQualityDiagnosis";

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
