// i18n-exempt: 无硬编码文案，全部走 t()（`absence` 是后端产的自由文本，与 `data_gaps` 同一处置：原样显示，不前端翻译）
/**
 * HorizonAgreementList —— 四档各自的「公式 vs LLM」对照行。
 *
 * 为什么单独一个组件、不复用 `HorizonDecisionStrip`：那一根摆的是**一侧的结论**，
 * 本组件摆的是**两侧之差**，且必须同时把两件容易被忽略的事说清楚 ——
 *   ① 逐档只有三维（方向 / 仓位 / 置信），与主档 `agreementBreakdown` 的五维分**不可并排比较**；
 *   ② 公式侧有部分腿的数值本来就来自 LLM 分析师的逐档判断（拍板 4）⇒ 一致不等于互相确认。
 * 这两句不写出来，四个 0-100 的数就会被人当成和主档同一把尺子。
 *
 * 「无从比对」与「0 分」必须分两样东西：`comparable=false` 时产端 `total` 为 null，
 * 本组件显示产端给的成因句；**不得**渲染成 0 分或空白格。
 *
 * B2 归因（`divergences`）就摆在各自那一档行内，而不是另起一张表：一条「谁把这一档压下去」
 * 离开这一档的两侧动作就没有解读上下文。驱动项是**判据码**，叙事全走 i18n；码表缺席
 * （产端新增值而前端没译）时 `defaultValue` 直出原码 —— 与主档归因同一处置。
 */
import type { HorizonAgreement, HorizonDivergence } from "@/types";
import { useTranslation } from "react-i18next";

interface HorizonAgreementListProps {
  agreements?: HorizonAgreement[] | null;
  /**
   * B2 逐档分歧归因（产端 `decision.divergenceByHorizon`，与 `agreements` 同序同档位键）。
   * 缺席 = 该记录产自归因上线前 ⇒ 整块不渲染（不补「无驱动项」，那会被读成「两侧一致」）。
   */
  divergences?: HorizonDivergence[] | null;
  /** 容器 test id；不传则不加属性 */
  testId?: string;
}

export function HorizonAgreementList({ agreements, divergences, testId }: HorizonAgreementListProps) {
  const { t } = useTranslation();

  if (!agreements || agreements.length === 0) {
    return (
      <div data-testid={testId} data-state="not-produced" className="text-sm" style={{ color: "var(--muted)" }}>
        {t("stockAnalysis.horizonAgreementNotProduced")}
      </div>
    );
  }

  const divOf = (tier: string) => divergences?.find((d) => d.tier === tier) ?? null;

  /**
   * 驱动项插值：一次给全参数袋，与各条 `t()` 只用得上自己那部分 —— 与主档
   * `crossCheck.driver.<码>`（DecisionBanner.tsx:2345）同一形态，不另立一套。
   * 腿名直出 `factor` 原文：档腿因子名与主档 `crossCheck.leg` 那 7 个显示名不是同一集合。
   */
  const driverParams = (d: HorizonDivergence) => ({
    gate: d.entryGate ?? "?",
    basis: d.gateBasis ?? "",
    from: d.veto?.[0] ?? "?",
    to: d.veto?.[1] ?? "?",
    legs: (d.negativeLegs ?? [])
      .map((l) =>
        t("stockAnalysis.horizonDivergence.legContribution", {
          leg: l[0],
          bias: l[1].toFixed(2),
          weight: l[2].toFixed(2),
        })
      )
      .join(" · "),
    absent: (d.absentLegs ?? []).join(" · "),
    n: (d.dataGaps ?? []).length,
    gaps: (d.dataGaps ?? []).join(" · "),
    why: d.llmAbsence ?? "",
  });

  return (
    <div data-testid={testId} data-state="ok" className="space-y-1 text-sm">
      <div className="font-medium" style={{ color: "var(--color-text-secondary)" }}>
        {t("stockAnalysis.horizonAgreementTitle")}
      </div>
      {agreements.map((h) => {
        const d = divOf(h.tier);
        return (
          <div
            key={h.tier}
            data-testid={`horizon-agreement-row-${h.tier}`}
            data-comparable={h.comparable ? "true" : "false"}
            className="flex items-center gap-2 flex-wrap"
          >
            <span className="w-14">{h.tierLabel}</span>
            {h.comparable
              ? (
                <>
                  <span className="font-mono">{h.formulaAction ?? "?"}</span>
                  <span style={{ color: "var(--muted)" }}>vs</span>
                  <span className="font-mono">{h.llmAction ?? "?"}</span>
                  <span className="font-mono font-medium">{h.total ?? 0}</span>
                  {h.conflictType && <span style={{ color: "var(--muted)" }}>{h.conflictType}</span>}
                  {typeof h.sharedEvidenceWeightPct === "number" && (
                    <span data-testid={`horizon-agreement-indep-${h.tier}`} style={{ color: "var(--muted)" }}>
                      {t("stockAnalysis.horizonAgreementIndependenceNote", {
                        pct: Math.round(h.sharedEvidenceWeightPct),
                      })}
                    </span>
                  )}
                </>
              )
              : <span data-testid={`horizon-agreement-absent-${h.tier}`}>{h.absence ?? ""}</span>}
            {d && (
              <div
                data-testid={`horizon-divergence-${h.tier}`}
                className="w-full space-y-0.5 text-[11px]"
                style={{ color: "var(--color-text-secondary)" }}
              >
                {d.drivers.length === 0
                  ? (
                    // 空清单 ≠ 两侧一致：产端只是没点出可归因的那一步
                    <span data-testid={`horizon-divergence-none-${h.tier}`}>
                      {t("stockAnalysis.horizonDivergence.none")}
                    </span>
                  )
                  : d.drivers.map((code) => (
                    <div
                      key={code}
                      data-testid={`horizon-driver-${h.tier}-${code}`}
                      style={{ color: "var(--muted)" }}
                    >
                      {t(`stockAnalysis.horizonDivergence.driver.${code}`, {
                        defaultValue: code,
                        ...driverParams(d),
                      })}
                    </div>
                  ))}
              </div>
            )}
          </div>
        );
      })}
      <div data-testid="horizon-agreement-dim-note" style={{ color: "var(--muted)" }}>
        {t("stockAnalysis.horizonAgreementDimNote")}
      </div>
    </div>
  );
}
