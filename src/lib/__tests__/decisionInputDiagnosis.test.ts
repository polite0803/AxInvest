// SPDX-License-Identifier: AGPL-3.0-only

import { buildDecisionInputsReport, summarizeDecisionInputs } from "@/lib/decisionInputDiagnosis";
import { describe, expect, it } from "vitest";

// i18n-exempt: 测试内断言值为模块内硬编码中文技术串（见 decisionInputDiagnosis.ts 文件头），非 UI 文案。
//
// ─────────────────────────────────────────────────────────────────────────────
// 背景（2026-09-20）：cls-risk-level 由 LlmClassifierNode 下沉为 Rhai CodeNode
// （`risk-level.rhai`，模板 v55）。
//
// 新脚本的降级契约与原 LLM 版**不同**：上游字段缺失时它**不报错**（保住整轮工作流），
// 而是 ① 置 `degraded = true`、② 把缺失项从 `metrics` 里**删掉**（不写哨兵值）、
// ③ 在 `warnings` 里列出不可用的输入名。
//
// 问题在于：这三个信号当时**一个消费方都没有** —— Rust 侧只取 `result.category`，
// 本模块只取 `category`，store 侧只读节点状态 ⇒ 降级只能事后查 `node_executions`。
// 「设计成可见、实际不可见」是比「静默失败」更难发现的一类缺陷。
//
// 本测试守的就是这条接线：**降级必须能在诊断条目里被看见**。
// ─────────────────────────────────────────────────────────────────────────────

/** 构造一个 CodeNode 形态的 `cls-risk-level` 输出（`{status, result, ...}`）。 */
const codeNodeRaw = (result: Record<string, unknown>) => ({
  node_id: "cls-risk-level",
  status: "completed",
  language: "rhai",
  result,
});

describe("cls-risk-level 降级信号可见性（v55 Rhai 下沉）", () => {
  it("degraded=true 时 note 必须带出降级与缺失项数", () => {
    const results = {
      "cls-risk-level": codeNodeRaw({
        category: "中风险",
        risk_factors: [],
        matched_rules: ["R-中-B"],
        reason: "按规则判定",
        degraded: true,
        warnings: ["risk_roe 缺失或非数值", "risk_gross_margin 缺失或非数值"],
        metrics: { sharpe: 0.403, roe_ttm_pct: null, gross_margin_pct: null },
      }),
    };

    const report = buildDecisionInputsReport(results, null);
    const item = report.find((r) => r.nodeId === "cls-risk-level");
    expect(item, "报告里应有 cls-risk-level 条目").toBeDefined();

    expect(item!.stance).toBe("中风险");
    expect(item!.note).toContain("确定性降级");
    expect(item!.note).toContain("2 项");
    // 原有的正面结论不能被降级文案吞掉 —— 降级 ≠ 分类失败
    expect(item!.note).toContain("分类已输出");
  });

  it("degraded 缺失（数据齐全 / 旧 LLM 版包装）⇒ note 保持原样，不引入噪声", () => {
    const results = {
      "cls-risk-level": codeNodeRaw({
        category: "低风险",
        matched_rules: ["R-低-A"],
        reason: "按规则判定",
        metrics: { sharpe: 0.8 },
      }),
    };

    const report = buildDecisionInputsReport(results, null);
    const item = report.find((r) => r.nodeId === "cls-risk-level");
    expect(item!.note).toBe("分类已输出");
    expect(item!.note).not.toContain("降级");
  });

  it("degraded=false 同样不产生降级文案（只有true 才算降级）", () => {
    const results = {
      "cls-risk-level": codeNodeRaw({ category: "高风险", degraded: false, warnings: [] }),
    };

    const report = buildDecisionInputsReport(results, null);
    const item = report.find((r) => r.nodeId === "cls-risk-level");
    expect(item!.note).toBe("分类已输出");
  });

  it("category 缺失时仍报字段缺失（降级文案可与之叠加）", () => {
    const results = {
      "cls-risk-level": codeNodeRaw({
        degraded: true,
        warnings: ["risk_volatility 缺失或非数值"],
      }),
    };

    const report = buildDecisionInputsReport(results, null);
    const item = report.find((r) => r.nodeId === "cls-risk-level");
    expect(item!.note).toContain("category 字段缺失");
    expect(item!.note).toContain("确定性降级");
  });

  it("warnings 非数组时不把 note 写成 NaN（容错）", () => {
    const results = {
      "cls-risk-level": codeNodeRaw({ category: "中风险", degraded: true, warnings: "缺了两个" }),
    };

    const report = buildDecisionInputsReport(results, null);
    const item = report.find((r) => r.nodeId === "cls-risk-level");
    expect(item!.note).toContain("确定性降级");
    expect(item!.note).toContain("0 项");
    expect(item!.note).not.toContain("NaN");
  });

  it("节点输出整体缺失 ⇒ status=missing（降级接线不影响既有判定）", () => {
    const report = buildDecisionInputsReport({}, null);
    const item = report.find((r) => r.nodeId === "cls-risk-level");
    expect(item!.status).toBe("missing");
    expect(item!.note).toBe("节点输出缺失");
  });

  it("降级不进 status 计数（它不属于 missing/low/untrusted/normal 四态）", () => {
    const results = {
      "cls-risk-level": codeNodeRaw({ category: "中风险", degraded: true, warnings: ["x"] }),
    };

    const report = buildDecisionInputsReport(results, null);
    const sum = summarizeDecisionInputs(report);
    // 降级只影响 note，不改变状态分布 —— 若将来要新增五态，本断言会失败，
    // 那是**预期信号**：届时需同步 DecisionBanner 的渲染与统计口径，别只改这里。
    expect(sum.total).toBe(report.length);
    expect(sum.missing + sum.low + sum.untrusted + sum.normal).toBe(sum.total);
    expect(sum.untrusted).toBe(0);
  });
});

// ─────────────────────────────────────────────────────────────────────────────
// 2026-10-09：逐档分析师实例（v133 `a-catalyst--{tier}`）的键解析。
//
// 用户实证：面板把「跑了且有输出」的催化剂分析师报成「节点输出缺失」（f3 行，权重 20%）。
// 根因是 `NODE_SPECS` 写 base id（`a-catalyst`）而黑板键是**按档实例**
// （`a-catalyst--mid`，`seed_stock_analysis.rs` 经 `analyst_node_id` 生成）
// ⇒ `results[baseId]` 恒 `undefined`。代表实例口径与 seed 的 `data-quality` 输入一致（先 mid）。
// 同时锁住「兜底标记必须按同一个命中键查」，否则会出现「值取到了、降级判成正常」。
// ─────────────────────────────────────────────────────────────────────────────
describe("逐档分析师实例的键解析（a-catalyst--{tier}）", () => {
  const catalyst = (level: string, confidence: number) => ({
    content: JSON.stringify({ verdict: { catalyst_level: level, confidence } }),
  });
  const item = (report: ReturnType<typeof buildDecisionInputsReport>) => report.find((r) => r.nodeId === "a-catalyst")!;

  it("黑板只有 base id 时照旧解析（历史快照 / 快速链）", () => {
    const report = buildDecisionInputsReport({ "a-catalyst": catalyst("L2", 55) }, null);
    expect(item(report).status).toBe("normal");
    expect(item(report).confidence).toBe(55);
    expect(item(report).stance).toBe("L2");
  });

  it("黑板只有按档实例时不再误报缺失，且优先取 mid 档", () => {
    const report = buildDecisionInputsReport({
      "a-catalyst--ultra_short": catalyst("L1", 40),
      "a-catalyst--short": catalyst("L2", 50),
      "a-catalyst--mid": catalyst("L3", 60),
      "a-catalyst--long": catalyst("L4", 70),
    }, null);
    expect(item(report).status).toBe("normal");
    expect(item(report).confidence).toBe(60);
    expect(item(report).stance).toBe("L3");
  });

  it("mid 档缺席时退任一可用档（只换代表实例，不猜数值）", () => {
    const report = buildDecisionInputsReport({ "a-catalyst--long": catalyst("L4", 70) }, null);
    expect(item(report).status).toBe("normal");
    expect(item(report).confidence).toBe(70);
  });

  it("四档全缺时仍报 missing（回退不得把真缺失洗白）", () => {
    const report = buildDecisionInputsReport({}, null);
    expect(item(report).status).toBe("missing");
    expect(item(report).note).toBe("节点输出缺失");
  });

  it("兜底标记按同一个命中键查：mid 档 __untrusted ⇒ status=untrusted", () => {
    const report = buildDecisionInputsReport(
      { "a-catalyst--mid": catalyst("L3", 60) },
      { "a-catalyst--mid": true },
    );
    expect(item(report).status).toBe("untrusted");
    expect(item(report).note).toContain("strict_mode 兜底");
  });

  it("负控：后缀不在四档域内不得被当成命中（回退只在四档域内发生）", () => {
    const report = buildDecisionInputsReport({ "a-catalyst--deca": catalyst("L9", 99) }, null);
    expect(item(report).status).toBe("missing");
  });
});
