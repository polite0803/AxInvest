// i18n-exempt: 无硬编码文案，全部走 t()
/**
 * HorizonConfusionBoard —— 逐档「公式判对否 × LLM 判对否」的 2×2 结局看板（B4）。
 *
 * 为什么不是四个大数字：四格的**分母是同一批样本**（同窗口、同判定函数，见产端
 * `build_horizon_confusion` 的结构体文档），但 `judged` 小于产端门槛时它们只是**计数**
 * 而不是结论 —— 所以「样本不足」必须与四格同屏，而不是等样本够了再说。
 *
 * 三条口径必须出现在界面上（缺一条就会被读成别的意思）：
 *   ① `llmUndecided` **不属于**任何一格（LLM 缺席 / 窗口未满 / 观望持有这类非方向性动作
 *      都不构成「LLM 判错」）；
 *   ② 四格只比**方向对错**，不代表收益高低，也不与 `byHorizon` 命中率同分母；
 *   ③ 档位显示名走本项目 i18n（复用 `HORIZON_T_SUFFIX`，不另立一张四档表）。
 */
import { HORIZON_T_SUFFIX } from "@/lib/stock-analysis-utils";
import type { HorizonConfusionRow } from "@/types";
import { useTranslation } from "react-i18next";

interface HorizonConfusionBoardProps {
  rows?: HorizonConfusionRow[] | null;
  /** 容器 test id；不传则不加属性 */
  testId?: string;
}

export function HorizonConfusionBoard({ rows, testId }: HorizonConfusionBoardProps) {
  const { t } = useTranslation();

  if (!rows || rows.length === 0) {
    return (
      <div data-testid={testId} data-state="not-produced" style={{ color: "var(--muted)", fontSize: 12 }}>
        {t("stockAnalysis.reflection.horizonConfusionNotProduced")}
      </div>
    );
  }

  return (
    <div data-testid={testId} data-state="ok" style={{ display: "flex", flexDirection: "column", gap: 10 }}>
      {rows.map((r) => {
        const suffix = HORIZON_T_SUFFIX[r.tier];
        const label = suffix ? t(`stockAnalysis.timeHorizon${suffix}`) : r.tier;
        return (
          <div
            key={r.tier}
            data-testid={`confusion-row-${r.tier}`}
            data-sufficient={r.sufficient ? "true" : "false"}
          >
            <div style={{ fontWeight: 500, fontSize: 13 }}>{label}</div>
            {!r.sufficient && (
              <div
                data-testid={`confusion-insufficient-${r.tier}`}
                style={{ color: "var(--muted)", fontSize: 12 }}
              >
                {t("stockAnalysis.reflection.horizonConfusionInsufficient", { judged: r.judged })}
              </div>
            )}
            <div className="font-mono" style={{ fontSize: 12 }}>
              {t("stockAnalysis.reflection.horizonConfusionCells", {
                both: r.bothCorrect,
                formulaOnly: r.formulaOnly,
                llmOnly: r.llmOnly,
                bothWrong: r.bothWrong,
              })}
            </div>
            {r.llmUndecided > 0 && (
              <div
                data-testid={`confusion-undecided-${r.tier}`}
                style={{ color: "var(--muted)", fontSize: 12 }}
              >
                {t("stockAnalysis.reflection.horizonConfusionUndecided", { undecided: r.llmUndecided })}
              </div>
            )}
            {typeof r.avgHorizonAgreement === "number" && (
              <div style={{ color: "var(--muted)", fontSize: 12 }}>
                {t("stockAnalysis.reflection.horizonConfusionAvgAgreement", {
                  pct: Math.round(r.avgHorizonAgreement),
                })}
              </div>
            )}
          </div>
        );
      })}
      <div style={{ color: "var(--muted)", fontSize: 12 }}>
        {t("stockAnalysis.reflection.horizonConfusionScopeNote")}
      </div>
    </div>
  );
}
