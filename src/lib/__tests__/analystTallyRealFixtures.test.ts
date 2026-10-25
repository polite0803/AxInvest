// i18n-exempt: 计票回归门，断言全为结构化输入/输出与源码文本，无 UI 文案。
//
// 夹具是 2026-10-10 立讯精密（002475）那条真实分析的两份分析师产物原文
// （stock_analyses.blackboard_snapshot），不是按期望答案反造的样本。
import { cleanToolCallTags, reconstructVerdictTag } from "@/lib/agentOutput";
import {
  classifyAnalystReport,
  classifySentiment,
  extractBullBearScores,
  normalizeVerdictScore,
  tallyAnalystSentiment,
} from "@/lib/stock-analysis-utils";
import { readFileSync } from "node:fs";
import path from "node:path";
import { describe, expect, it } from "vitest";

const fixture = (name: string) => readFileSync(path.resolve(__dirname, "fixtures", name), "utf8");

/** 资金面（中期）：VERDICT 标签里的 JSON 多一个右花括号 ⇒ 主解析路径必抛。 */
const hotMoneyMid = fixture("analyst-report-002475-hot-money-mid.txt");
/** 巴菲特估值（长线）：store 曾把它落成未重构的 strict_mode 裸 JSON（无 VERDICT 标签）。 */
const valueLong = fixture("analyst-report-002475-value-long.txt");

describe("真实产物回归锁：畸形 VERDICT 仍要计对票", () => {
  it("夹具前提自证：该份的 VERDICT JSON 确实解析失败（被修好就该换样本，不是删判据）", () => {
    const vIdx = hotMoneyMid.indexOf("<!-- VERDICT:");
    expect(vIdx).toBeGreaterThan(-1);
    const body = hotMoneyMid.slice(vIdx + "<!-- VERDICT:".length).split("-->")[0].trim();
    expect(() => JSON.parse(body)).toThrow();
  });

  it("靠裸分数抢救到 38/62 ⇒ 记看空（旧顶部条在此落到关键词兜底，记成中性）", () => {
    expect(extractBullBearScores(hotMoneyMid)).toEqual({ bull: 38, bear: 62 });
    expect(classifyAnalystReport(hotMoneyMid)).toBe("bearish");
  });
});

describe("真实产物回归锁：value 槽位的 verdict", () => {
  it("该份自己的裁决是「观望」52/48 ⇒ 中性，不是正文措辞带出来的看多", () => {
    const inner = (JSON.parse(valueLong) as { verdict: Record<string, unknown> }).verdict;
    expect(inner.verdict).toBe("观望");
    expect(inner.bull_score).toBe(52);
    expect(inner.bear_score).toBe(48);
    expect(classifyAnalystReport(valueLong)).toBe("neutral");
  });

  it("重构后走 VERDICT 主解析路径（不再依赖正则抢救）", () => {
    const reconstructed = reconstructVerdictTag(valueLong);
    expect(reconstructed).toContain("<!-- VERDICT:");
    expect(extractBullBearScores(reconstructed)).toEqual({ bull: 52, bear: 48 });
    expect(classifyAnalystReport(reconstructed)).toBe("neutral");
  });

  it("classifySentiment 单独调用也要读得到嵌套 verdict（此前 String(对象) 退化成全文扫描）", () => {
    expect(classifySentiment(cleanToolCallTags(valueLong))).toBe("neutral");
  });
});

describe("空产出不计票", () => {
  it("空串 / 纯空白既不算中性票，也不进 total", () => {
    expect(tallyAnalystSentiment(["", "   ", "\n\t"])).toEqual({
      bullish: 0,
      bearish: 0,
      neutral: 0,
      total: 0,
    });
    // 负控：有实产出时照常计票，证明上面的 0 来自「剔除」而不是「整体失灵」
    expect(tallyAnalystSentiment(["", "建议卖出，看空。", "   "])).toEqual({
      bullish: 0,
      bearish: 1,
      neutral: 0,
      total: 1,
    });
  });
});

describe("嵌套 verdict 下钻（结构化优先于正文反方向词）", () => {
  it("结构化说看多、正文全是空头措辞 ⇒ 以结构化为准", () => {
    const raw = JSON.stringify({ report: "主力资金流出，空头排列，趋势下行。", verdict: { stance: "看多" } });
    expect(classifySentiment(raw)).toBe("bullish");
  });

  it("只有嵌套分数、正文无方向词 ⇒ 从嵌套取分（旧实现落全文扫描 ⇒ 中性）", () => {
    expect(classifySentiment('{"verdict":{"bull_score":90,"bear_score":10}}')).toBe("bullish");
  });
});

describe("形态门：计票判据不得再出现第二份副本", () => {
  const uiFiles = {
    顶部共识条: "../../components/stock-analysis/StockAnalysisPage.tsx",
    分歧卡: "../../components/stock-analysis/AnalystReportGrid.tsx",
    紧凑摘要: "../../components/stock-analysis/dual-view/CompactAnalystSummary.tsx",
  };
  const src = (p: string) => readFileSync(path.resolve(__dirname, p), "utf8");

  /**
   * 单点计票入口三个都算：直接 `tallyAnalystSentiment`、逐份 `classifyAnalystReport`
   * （紧凑摘要要出明细），以及 `useAnalystConsensusByTier`（逐档共识，内部就是 tally）。
   * 判据锁的是「不在 UI 里再写一份数票循环」，不是锁某一个函数名。
   */
  const SINGLE_SOURCE_MARKERS = [
    "tallyAnalystSentiment(",
    "classifyAnalystReport(",
    "useAnalystConsensusByTier(",
  ];
  const usesSingleSource = (text: string) => SINGLE_SOURCE_MARKERS.some((m) => text.includes(m));

  it("三处 UI 都必须走单点判据（tally / 逐份 classify / 逐档共识 hook）", () => {
    for (const [name, p] of Object.entries(uiFiles)) {
      const text = src(p);
      expect(text.length, `${name} 源码应可读`).toBeGreaterThan(500);
      expect(usesSingleSource(text), `${name} 不得自带计票副本`).toBe(true);
    }
  });

  it("负控：自带数票循环、且不引用任何单点入口的源码必须判为副本", () => {
    const fakeCopy = 'let bullish = 0; for (const r of reports) { if (r.includes("看多")) { bullish++; } }';
    expect(usesSingleSource(fakeCopy)).toBe(false);
  });

  it("逐档共识 hook 必须透传 tally，自己不得成为第四份计票实现", () => {
    const text = src("../../components/stock-analysis/useAnalystConsensusByTier.ts");
    expect(text.length, "hook 源码应可读").toBeGreaterThan(500);
    expect(text).toContain("tallyAnalystSentiment(");
    expect(text).not.toMatch(/bullish\+\+/);
  });

  it("交叉验证徽章显示票数，不得把加权和渲染成 N/M", () => {
    // 两个计数域：`consensus.votes.*` 是票数，`consensus.neutral/total` 是按持有期加权的分数和。
    // 后者一度被写成 `${neutral}/${total}` 显示 ⇒ 用户看到的是「3.3/11.6 中性」这种假票数，
    // 与同屏顶部条的真票数长得一样却不可比。
    const text = src("../../components/stock-analysis/RecommendationPanel.tsx");
    expect(text).toContain("consensus.votes.neutral");
    expect(text).not.toMatch(/\$\{consensus\.neutral\}\/\$\{consensus\.total\}/);
  });

  it("自己循环计票的调用侧必须处理「空产出＝无票」的 null 契约", () => {
    // 紧凑摘要要出逐份明细，故不走 tally；漏掉 null 分支就会把空产出记成中性票。
    const text = src(uiFiles.紧凑摘要);
    expect(text).toContain("classifyAnalystReport(");
    expect(text).toContain("=== null");
  });

  it("store 里 value 槽位写入 analystReports 的**每一处**都必须过 reconstructVerdictTag", () => {
    // 三条写入路径（实时 routeNodeOutput ×2 / parseWorkflowResults / loadAnalysis）
    // 漏一条 ⇒ live 与重载算出的票型不同源，正是本次修的缺陷。
    const store = src("../../stores/feature/stockAnalysisStore.ts");
    const valueWrites = store.split(/\r?\n/).filter((line) => {
      const t = line.trim();
      return (t.startsWith("analystReports[viKey] =") || t.startsWith("reports[viKey] =")
        || (t.includes("analystReports: { ...s.analystReports, [viKey]:")))
        && !t.startsWith("//");
    });
    // 下限断言：夹具面积不能缩到 0（写入点被改名会让本门静默空跑）
    expect(valueWrites.length).toBeGreaterThanOrEqual(4);
    for (const line of valueWrites) {
      expect(line, "value 槽位落 analystReports 前必须重构 VERDICT 标签").toContain("reconstructVerdictTag(");
    }
  });

  it("除分歧卡（表格逐行需自读分数）外，UI 里不得出现 VERDICT 解析或 1.2 倍阈值", () => {
    for (const name of ["顶部共识条", "紧凑摘要"] as const) {
      const text = src(uiFiles[name]);
      expect(text, `${name} 不得内联 VERDICT 解析`).not.toContain('indexOf("<!-- VERDICT:")');
      expect(text, `${name} 不得内联 1.2 倍阈值`).not.toContain("* 1.2");
    }
  });

  it("分数标度还原只许走 normalizeVerdictScore，UI 侧不得再出现 ×10 / ≤10 猜测", () => {
    // 实测最近 400 条分析 2846 个分数：2835 个 >10 ⇒ 提示词声明的 0-100 才是真实形态。
    // 此前卡片标签与表格列各写了一份「≤10 就 ×10」，同一条报告在卡片显示 70、
    // 在共识计票里按 7 算（且把声明值 7% 编成 70%）。
    for (const [name, p] of Object.entries(uiFiles)) {
      const text = src(p);
      expect(text, `${name} 不得内联 ≤10 猜测`).not.toContain("<= 10 ?");
      expect(text, `${name} 不得内联 ×10 还原`).not.toContain("* 10 :");
      expect(text, `${name} 不得内联 >1 判据`).not.toContain("> 1 ?");
    }
    // 面积下限：引用点被改名/删掉时，本门不得静默退化成空跑
    const refs = [uiFiles.分歧卡, uiFiles.顶部共识条, "../../components/stock-analysis/AnalystReportCard.tsx"]
      .reduce((acc, p) => acc + (src(p).match(/normalizeVerdictScore\(/g) ?? []).length, 0);
    expect(refs).toBeGreaterThanOrEqual(3);
  });
});

describe("normalizeVerdictScore：声明标度 0-100，只还原 ≤1 的小数形态", () => {
  it("按标度表逐档给值", () => {
    expect(normalizeVerdictScore(0.62)).toBe(62);
    expect(normalizeVerdictScore(1)).toBe(100);
    expect(normalizeVerdictScore(1.5)).toBe(1.5);
    expect(normalizeVerdictScore(7)).toBe(7); // 声明值 7% —— 不猜成 70
    expect(normalizeVerdictScore(52)).toBe(52);
  });

  it("0-1 分制与 0-100 分制的同向判决必须一致（计票阈值不受标度影响）", () => {
    expect(extractBullBearScores('<!-- VERDICT: {"bull_score":0.62,"bear_score":0.38} -->'))
      .toEqual(extractBullBearScores('<!-- VERDICT: {"bull_score":62,"bear_score":38} -->'));
  });
});
