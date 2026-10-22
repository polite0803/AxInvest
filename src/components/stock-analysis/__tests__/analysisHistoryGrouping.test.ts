import { readFileSync } from "node:fs";
import path from "node:path";
import { describe, expect, it } from "vitest";
import { groupAnalysisHistory } from "../../../lib/stock-analysis-utils";

const rec = (stockCode: string, stockName: string, createdAt: number) => ({
  stockCode,
  stockName,
  createdAt,
});

describe("历史分析按股票代码分组", () => {
  it("最新记录空名不得抹掉组名（取组内第一条非空名称）", () => {
    const groups = groupAnalysisHistory([
      // 现网形态：补分析 replay 行最新且名字为空，老记录有真名
      rec("000530", "", 200),
      rec("000530", "冰山冷热", 100),
    ]);
    expect(groups).toHaveLength(1);
    expect(groups[0].stockName).toBe("冰山冷热");
    expect(groups[0].items).toHaveLength(2);
  });

  it("全组都空名时组名保持空串（UI 只显示代码，不编造名称）", () => {
    const groups = groupAnalysisHistory([rec("000993", "", 10)]);
    expect(groups[0].stockName).toBe("");
  });

  it("组内按 createdAt 倒序，不同代码各成一组", () => {
    const groups = groupAnalysisHistory([
      rec("000530", "冰山冷热", 100),
      rec("000993", "闽东电力", 50),
      rec("000530", "冰山冷热", 300),
    ]);
    expect(groups.map((g) => g.stockCode)).toEqual(["000530", "000993"]);
    expect(groups[0].items.map((r) => r.createdAt)).toEqual([300, 100]);
  });

  it("历史弹窗与详情页列表共用同一分组函数（防再次各写一份）", () => {
    const root = path.resolve(import.meta.dirname, "..", "..", "..", "..");
    for (
      const file of [
        "src/components/stock-analysis/AnalysisHistoryButton.tsx",
        "src/components/stock-analysis/HistoricalAnalysisPanel.tsx",
      ]
    ) {
      expect(readFileSync(path.join(root, file), "utf8"), file).toContain("groupAnalysisHistory(");
    }
  });
});
