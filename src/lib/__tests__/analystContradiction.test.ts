// i18n-exempt: 断言全部为结构化诊断字段，无 UI 文案。
import { contradictionBasesForTier } from "@/lib/dataQualityDiagnosis";
import type { DataQualityDiagItem, DataQualityReport } from "@/types";
import { describe, expect, it } from "vitest";

function diag(over: Partial<DataQualityDiagItem> = {}): DataQualityDiagItem {
  return {
    name: "技术面分析师",
    expected_data: "",
    confidence: 70,
    status: "normal",
    gap_reason: "",
    // 新字段必须显式存在（空串＝后端判过且没有矛盾）——缺字段是「不知道」，语义完全不同
    inconsistent_reason: "",
    ...over,
  } as DataQualityDiagItem;
}

function report(over: Partial<DataQualityReport> = {}): DataQualityReport {
  return { grade: "A", score: 90, diagnostics: {}, ...over } as unknown as DataQualityReport;
}

describe("contradictionBasesForTier", () => {
  it("代表实例判出矛盾 ⇒ 返回去 a- 前缀的 base（与 store 落键同形态）", () => {
    const r = report({
      diagnostics: {
        mk: diag({ inconsistent_reason: "verdict='看多' 但 bull_score(30) <= bear_score(60)" }),
        sent: diag(),
      },
    });
    expect(contradictionBasesForTier(r, "mid")).toEqual(new Set(["market-analyst"]));
  });

  it("逐档行优先于代表实例：只有本档矛盾才扣本档", () => {
    const r = report({
      diagnostics: { mk: diag() },
      diagnostics_by_tier: {
        mk: {
          mid: diag(),
          short: diag({ inconsistent_reason: "verdict='看空' 但 bear_score(20) <= bull_score(70)" }),
        },
      },
    });
    expect(contradictionBasesForTier(r, "short")).toEqual(new Set(["market-analyst"]));
    expect(contradictionBasesForTier(r, "mid")).toEqual(new Set());
  });

  it("该档没有逐档行时回落代表实例，不静默当成无矛盾", () => {
    const r = report({
      diagnostics: { mk: diag({ inconsistent_reason: "verdict='看多' 但 bull_score(10) <= bear_score(90)" }) },
      diagnostics_by_tier: { mk: { mid: diag() } },
    });
    // long 档没行 ⇒ 用代表实例那一行（与后端 grade 只看代表实例同一取舍）
    expect(contradictionBasesForTier(r, "long")).toEqual(new Set(["market-analyst"]));
    // mid 有行且判过没矛盾 ⇒ 空集合
    expect(contradictionBasesForTier(r, "mid")).toEqual(new Set());
  });

  it("旧快照没有这个字段 ⇒ null（不知道），调用方必须照常计票", () => {
    const legacy = report({
      diagnostics: {
        mk: {
          name: "技术面分析师",
          expected_data: "",
          confidence: 70,
          status: "normal",
          gap_reason: "",
        } as unknown as DataQualityDiagItem,
      },
    });
    expect(contradictionBasesForTier(legacy, "mid")).toBeNull();
    expect(contradictionBasesForTier(null, "mid")).toBeNull();
    expect(contradictionBasesForTier(undefined, "mid")).toBeNull();
  });

  it("负控：字段存在且全为空串时必须返回空集合而不是 null", () => {
    const clean = report({ diagnostics: { mk: diag(), sent: diag() } });
    const got = contradictionBasesForTier(clean, "mid");
    expect(got).not.toBeNull();
    expect(got!.size).toBe(0);
  });
});
