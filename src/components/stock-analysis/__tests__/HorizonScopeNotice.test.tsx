import { render } from "@testing-library/react";
import { readdirSync, readFileSync } from "node:fs";
import path from "node:path";
import { describe, expect, it, vi } from "vitest";
import { HorizonScopeNotice } from "../HorizonScopeNotice";

vi.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}));

/**
 * 「本环节四档共用」声明的门（PLAN §五十三 ⑤ 甲）。
 *
 * 要挡住的不是「少了一句说明」，而是两种复发形态：
 *   ① **三组共用一条占位串** —— 上一轮「卡片写着『辩手』却是共用占位串泄漏」就是这一族，
 *      读者从文案上分不出自己看的是分析师、辩论还是风险；
 *   ② **面板被改回装作按档**（把同一份内容渲染四遍）或说明被整块删掉 ——
 *      于是「四档决策并列 + 一份风险」又变成无从解释的矛盾。
 * ② 由源码级断言守（三个宿主各必须挂上自己那一格），①由「三格 key 互不相同」守。
 */
// __tests__ → stock-analysis → components → src → 仓库根
const ROOT = path.resolve(import.meta.dirname, "..", "..", "..", "..");
const HOSTS: Array<[string, string]> = [
  ["src/components/stock-analysis/AnalystReportGrid.tsx", "analyst"],
  ["src/components/stock-analysis/DebatePanel.tsx", "debate"],
  ["src/components/stock-analysis/RiskMatrix.tsx", "risk"],
];

describe("HorizonScopeNotice", () => {
  it("三组各用自己的 key，互不相同（不得共用一条占位串）", () => {
    const keys = (["analyst", "debate", "risk"] as const).map(
      (scope) => render(<HorizonScopeNotice scope={scope} />).container.textContent ?? "",
    );
    expect(new Set(keys).size).toBe(3);
    expect(keys).toEqual([
      "stockAnalysis.horizonScopeAnalyst",
      "stockAnalysis.horizonScopeDebate",
      "stockAnalysis.horizonScopeRisk",
    ]);
  });

  it("三个宿主面板各自挂上对应那一格", () => {
    for (const [file, scope] of HOSTS) {
      const src = readFileSync(path.join(ROOT, file), "utf8");
      expect(src, `${file} 应引入 HorizonScopeNotice`).toContain("HorizonScopeNotice");
      expect(src, `${file} 应挂 scope="${scope}" 的共用声明`)
        .toContain(`<HorizonScopeNotice scope="${scope}" />`);
    }
  });

  it("负控：宿主被摘掉声明时，上一条断言真的会红", () => {
    const [file, scope] = HOSTS[0];
    const src = readFileSync(path.join(ROOT, file), "utf8")
      .replace(`<HorizonScopeNotice scope="${scope}" />`, "");
    expect(src).not.toContain(`<HorizonScopeNotice scope="${scope}" />`);
  });

  /**
   * v128（B1）后 risk 那格的**双向**判据（PLAN §五十四 ④：声明与数据层事实一致）。
   *
   * 两种谎报都要挡：
   *   · 写窄（继续说「四档共用」）—— 风险已按档，读者会把四档差异当成复制品；
   *   · 写宽（顺势说「风险已全按档」）—— 本版只有**一根**轴按档，波动率/夏普/基本面仍是全局。
   * 故既锁「提到了按档的那根轴」，也锁「点名了仍是全局的三条轴」。
   *
   * 权威源是 `risk-level.rhai` 输出的 `axisScope`（图形态由 Rust 侧
   * `seeded_template_carries_horizon_scoped_risk_nodes` 锁）；本测试把那份轴表与文案逐字对上，
   * 改判据不同批改文案就会红（反手抄门）。
   *
   * **debate 一格已于 v161（A4）反向收口**：辩论按四档展开后，旧声明「四档共用同一份」
   * 必须撤下 —— 与 analyst 同款：旧句降级为负控，11 语言以语言中立标记 `bull` 锁漏改。
   *
   * **analyst 一格已于 2026-10-16 反向收口**：v133（B2-2）起分析师实例由
   * `Period::analyst_subset()` 逐档展开，而本格文案长期仍写「四档共用同一批分析师输出」——
   * 上一版这条断言把那句谎**锁成了期望值**（`toContain("四档共用")`），于是门反过来挡住了修正本身。
   * 现改为：analyst 必须声明按档，且**与被声明的事实同源核对**（读种子现场确认逐档展开仍在发生），
   * 被锁的旧文案降级成负控样本。
   */
  it("v129 的 risk 声明与 axisScope 事实一致，analyst / debate 均已按档", () => {
    // 谓词只写一次，正样本与变异样本走同一条判据（否则「负控」只是把字符串再说一遍）
    const riskClaimIsHorizonScoped = (text: string) =>
      text.includes("回撤深度")
      && text.includes("风险偏置")
      && text.includes("仓位上限")
      && text.includes("波动率")
      && text.includes("夏普")
      && text.includes("基本面")
      && !text.includes("四档共用");

    const zh = JSON.parse(
      readFileSync(path.join(ROOT, "src/i18n/locales/zh-CN.json"), "utf8"),
    ).stockAnalysis;
    expect(riskClaimIsHorizonScoped(zh.horizonScopeRisk)).toBe(true);

    const rhai = readFileSync(path.join(ROOT, "src-tauri/src/commands/risk-level.rhai"), "utf8");
    expect(rhai).toContain('"tierWindow"');
    expect(rhai).toContain('"global60"');
    expect(rhai).toContain('"latestReport"');

    // analyst：声明必须与数据层事实同向（v133 B2-2 起逐档展开）。谓词同样只写一次，正负样本共用。
    // v161 起辩论也按档 ⇒ 旧尾部那句「仍然共用一份的是辩论」不得回流。
    const analystClaimIsHorizonScoped = (text: string) =>
      text.includes("已按档展开")
      && text.includes("分析师子集")
      && !text.includes("四档共用")
      && !text.includes("共用一份的是辩论");
    expect(analystClaimIsHorizonScoped(zh.horizonScopeAnalyst)).toBe(true);
    // 负控：被本轮撤下的那句谎（v133 之前的现网文案）必须被同一谓词拒掉
    expect(
      analystClaimIsHorizonScoped("分析师报告：本轮四档共用同一批分析师输出；按档独立的是评分、决策与出场。"),
    ).toBe(false);
    // 负控 2：只说「已按档」而不点名子集来源与仍共用的那一格（写宽的那类谎报）同样拒掉
    expect(analystClaimIsHorizonScoped("分析师报告：已按档展开。")).toBe(false);

    // 事实侧同判据：种子现场仍在逐档展开实例 —— 声明与实现缝在同一来源上，
    // 分析师若退化回单份，本条与文案判据一起红（而不是留着一句正确的谎）。
    const seed = readFileSync(
      path.join(ROOT, "src-tauri/src/commands/stock_analysis_setup/seed_stock_analysis.rs"),
      "utf8",
    );
    expect(seed).toContain("p.analyst_subset().contains(id)");

    // debate：v161 起逐档辩论已落地（每档一组 `bull-rN--<档>` / `bear-rN--<档>` 节点）
    const debateClaimIsHorizonScoped = (text: string) =>
      text.includes("bull")
      && text.includes("按档")
      && !text.includes("四档共用")
      && !text.includes("同一份");
    expect(debateClaimIsHorizonScoped(zh.horizonScopeDebate)).toBe(true);
    // 负控：v161 之前那句现网文案必须被同一谓词拒掉
    expect(
      debateClaimIsHorizonScoped("辩论：本轮只进行一轮跨视角辩论，四档共用同一份辩论记录。"),
    ).toBe(false);

    // 11 语言同批改口：新句每种语言都带语言中立标记（analyst=`VERDICT`，debate=`bull`），
    // 旧句没有 ⇒ 漏改即红。（清单由目录现场列出，不手抄 11 个文件名。）
    const localeFiles = readdirSync(path.join(ROOT, "src/i18n/locales"))
      .filter((f) => f.endsWith(".json"));
    expect(localeFiles.length).toBeGreaterThanOrEqual(11);
    for (const f of localeFiles) {
      const data = JSON.parse(
        readFileSync(path.join(ROOT, "src/i18n/locales", f), "utf8"),
      ).stockAnalysis;
      expect(data.horizonScopeAnalyst, `${f} 未随本轮改口`).toContain("VERDICT");
      expect(data.horizonScopeDebate, `${f} 未随逐档辩论改口`).toContain("bull");
    }

    // 负控：改回 v127 那句「四档共用」的文案必须被同一谓词拒掉
    expect(
      riskClaimIsHorizonScoped("风险评估：本轮产出一份风险分类，四档共用（仓位上限亦由此约束）。"),
    ).toBe(false);
    // 负控 2：只说按档、不点名仍是全局的三条轴（写宽的那类谎报）同样拒掉
    expect(riskClaimIsHorizonScoped("风险分类：四档各自成档。")).toBe(false);
    // 负控 3（v129）：v128 那句**当时正确**的文案 —— 只点名回撤深度与三条全局轴、没有主链
    // 按档那半句 —— 在 v129 之后必须被拒；否则谓词就查不出「声明落后于数据层一个版本」这类腐烂。
    expect(
      riskClaimIsHorizonScoped(
        "风险分类：已按档判据 —— 每档用本档持仓窗口的回撤深度单独分级；波动率、夏普比率与基本面四项仍是 60 日全局 / 最新财报口径。",
      ),
    ).toBe(false);
  });
});
