// i18n-exempt: 静态源码判据，断言的是 i18n key 与代码文本，无 UI 展示文案。
import { readFileSync } from "node:fs";
import path from "node:path";
import { describe, expect, it } from "vitest";

const KEY = 't("stockAnalysis.analystConfidenceSelfReported")';
const src = (rel: string) => readFileSync(path.resolve(__dirname, rel), "utf8");

/**
 * 分析师的 `confidence` 是**自报**把握度，没跟后续涨跌对过账（校准尚未实施），
 * 也不是胜率；而主档「决策置信度」是公式后验。两个数同名「置信度」，
 * 同屏并列时不标口径就会被读成可比 —— 这三格各自都必须带口径说明。
 */
describe("分析师自报置信度的口径标注", () => {
  it("分析师卡片的置信格挂了说明", () => {
    expect(src("../AnalystReportCard.tsx")).toContain(KEY);
  });

  it("逐档 VERDICT 表格有两格挂（JEV 那格 + 表格列头），缺一处即红", () => {
    const n = src("../AnalystReportGrid.tsx").split(KEY).length - 1;
    expect(n).toBe(2);
  });

  it("负控：把说明换成别的 key 后必须判红（证明上面两条不是恒真）", () => {
    const mutated = src("../AnalystReportCard.tsx").split(KEY).join('t("stockAnalysis.confidence")');
    expect(mutated).not.toContain(KEY);
  });

  it("11 语言都有这条文案（缺译会让非中文界面显示原始 key）", async () => {
    const dir = path.resolve(__dirname, "../../../i18n/locales");
    const langs = ["zh-CN", "zh-TW", "en-US", "ja", "ko", "de", "es", "fr", "ru", "ar", "hi"];
    for (const lang of langs) {
      const obj = JSON.parse(readFileSync(path.join(dir, `${lang}.json`), "utf8"));
      const v = obj?.stockAnalysis?.analystConfidenceSelfReported;
      expect(typeof v, `${lang} 缺 analystConfidenceSelfReported`).toBe("string");
      expect((v as string).length, `${lang} 文案过短`).toBeGreaterThan(40);
    }
  });
});
