import { readFileSync } from "node:fs";
import path from "node:path";
import { describe, expect, it } from "vitest";
import { versionStatusKey } from "../AnalysisHistoryButton";

/**
 * 「历史分析列表看得出厂代」的门（用户 2026-10-09 报：列表不显示工作流版本，
 * 而本仓 `TEMPLATE_VERSION` 近期从 129 推到 143，落差全靠人肉查库才发现）。
 *
 * 要挡住的两类复发：
 *   ① **四态被压成两态**（「一致 / 不一致」）—— 库里**高于**代码正是本仓真实事故形态
 *     （版本门 `existing >= TEMPLATE_VERSION ⇒ 跳过`，三批改动一字不落库），
 *     与「落后」并成一句就看不出方向，也就看不出「这次启动根本不会重播种」；
 *   ② **`dbVersion = null` 参与差值计算** —— 那会把「库里没这张图」渲染成「落后 N 代」。
 *
 * 逐条给可复核读数：`t()` 在这里被 mock 成「返回 key 本身」，故断言的是**接线**而不是译文；
 * 译文由 i18n 三关（parity / placeholder / untranslated）与下面的 locales 断言各自守。
 */
const ROOT = path.resolve(import.meta.dirname, "..", "..", "..", "..");
const COMPONENT = "src/components/stock-analysis/AnalysisHistoryButton.tsx";

describe("AnalysisHistoryButton 的图代呈现", () => {
  it("四种状态各占一个 key，互不相同", () => {
    const keys = [
      versionStatusKey({ codeVersion: 143, dbVersion: 143 }),
      versionStatusKey({ codeVersion: 143, dbVersion: 137 }),
      versionStatusKey({ codeVersion: 143, dbVersion: 149 }),
      versionStatusKey({ codeVersion: 143, dbVersion: null }),
    ];
    expect(new Set(keys).size).toBe(4);
    expect(keys).toEqual([
      "stockAnalysis.templateVersion.matched",
      "stockAnalysis.templateVersion.behind",
      "stockAnalysis.templateVersion.ahead",
      "stockAnalysis.templateVersion.notSeeded",
    ]);
  });

  it("负控：codeVersion 为 0 时 dbVersion=null 仍判「未播种」，不被算成落后", () => {
    expect(versionStatusKey({ codeVersion: 0, dbVersion: null }))
      .toBe("stockAnalysis.templateVersion.notSeeded");
    // 同为「库里 0 / 代码 0」时是**一致**，与上一条不得混用同一分支
    expect(versionStatusKey({ codeVersion: 0, dbVersion: 0 }))
      .toBe("stockAnalysis.templateVersion.matched");
  });

  it("负控：库里高于代码不得落到 behind 那一支", () => {
    // 判据是 `dbVersion < codeVersion`；把它写成 `!==` 就会把「跳过重播种」说成「将要重播种」
    expect(versionStatusKey({ codeVersion: 143, dbVersion: 144 }))
      .toBe("stockAnalysis.templateVersion.ahead");
  });

  it("组件里三处接线都在：取数命令、状态文案、逐行标签", () => {
    const src = readFileSync(path.join(ROOT, COMPONENT), "utf8");
    expect(src).toContain("get_stock_template_version_status");
    expect(src).toContain("versionStatusKey(versionStatus)");
    // 逐行标签的缺席分支必须显式判 null（不得 `||` 把 0 也吞成「未知」）
    expect(src).toContain("r.templateVersion == null");
    expect(src).toContain("stockAnalysis.templateVersion.unknown");
    // 快速链不得打这枚标签：它的 template_version 是另一套计数（恒 1 =「派生资产第 1 版」），
    // 与主图的 v143 并排＝伪造可比性（2026-10-09 现网读数：库里根本没有 fast 行，
    // 一旦有就会显示成 v1）
    expect(src).toContain(`{r.templateId !== FAST_TEMPLATE_ID && (`);
  });

  it("zh-CN 的五条文案齐备，且 behind/ahead 带差值槽", () => {
    const zh = JSON.parse(
      readFileSync(path.join(ROOT, "src/i18n/locales/zh-CN.json"), "utf8"),
    ) as { stockAnalysis: { templateVersion: Record<string, string> } };
    const obj = zh.stockAnalysis.templateVersion;
    expect(Object.keys(obj).sort()).toEqual([
      "ahead",
      "behind",
      "matched",
      "notSeeded",
      "unknown",
    ]);
    for (const k of ["behind", "ahead"]) {
      expect(obj[k], `${k} 应带 {{gap}} 槽`).toContain("{{gap}}");
    }
    // 残缺槽（{{gap} 少一个右花括号）会被 i18next 当字面量显示 ⇒ 摘掉合法槽后不得再有余括号
    for (const [k, v] of Object.entries(obj)) {
      const stripped = v
        .replaceAll("{{db}}", "")
        .replaceAll("{{code}}", "")
        .replaceAll("{{gap}}", "");
      expect(stripped, `${k} 含残缺插值槽`).not.toMatch(/\{\{|\}\}/);
    }
  });
});
