// i18n-exempt: 无硬编码文案，全部走 t()（`dataGaps` 是产端自由文本，与主决策卡同一处置：原样显示，不前端翻译）
/**
 * F7ReferenceComparison —— 「f7 口径对照」块（v183）。
 *
 * 摆什么：同一个公式在**两种 f7 输入口径**下各自算出的结论之差。
 *   · 生产侧 `portfolio-mgr`：f7 = 分析师结构票（净票 × 离散度折减），不读 LLM 决策；
 *   · 参照侧 `portfolio-mgr-f7-ref`：f7 = trader 的方向/置信/报价折算，即 2026-10-25 之前的口径。
 * ⇒ 两侧之差就是「交易观点进入公式后造成的位移」，是一把判据，不是第二套评分体系。
 *
 * 为什么不并进上方「公式 vs LLM」那两张卡：那一对比的是**两条链互不认同**；本块比的是
 * **同一个公式换口径后的位移**。并成三列表，用户会把后者也读成「LLM 不同意公式」。
 * 生产侧的方向/置信/仓位只进差值句，不重复渲染整列。
 *
 * 缺席各占一句，**不得**压成 0 或空白：
 *   ① 键不存在 = 该记录产自 v183 之前（或参照节点未跑）⇒ 说「未产出」；
 *   ② 两侧 f7 口径**相同** ⇒ 说「对照无位移」，否则一个 0 差会被读成「交易观点不影响结论」。
 *      v185 起两侧形态**恒定**（生产恒 `analyst_votes`、参照恒 `trader`），故同口径只有两种成因：
 *      生产侧走了降级路径（两侧都 `absent`），或该记录产自 v183 时期、回滚位曾把生产侧改成
 *      trader 口径（旧记录）—— 后者由 `f7Source` 逐条读出，不由本块猜测；
 *   ③ `weightsCollapsed=true` ⇒ 参照侧结论不可用；
 *   ④ 方向档是权威哨兵（`数据缺失`）⇒ 说「未给出方向结论」，不套档位标签。
 * 数值缺位 ⇒ 不算差值，也不显示 0 差（0 差＝宣称两侧一致）。
 */
import { getActionTKey, getRiskTKey, isAbsentDecisionAction, parseAction } from "@/lib/stock-analysis-utils";
import type { F7ReferenceSnapshot } from "@/types";
import { useTranslation } from "react-i18next";

interface F7ReferenceComparisonProps {
  /** 参照侧快照；`null`/`undefined` = 该记录没跑参照口径 ⇒ 说「未产出」，不渲染空卡 */
  snapshot?: F7ReferenceSnapshot | null;
  /** 生产侧（主决策）的 f7 口径与三个数：差值句要用它们，本块不重复渲染生产列 */
  productionF7Source?: string | null;
  productionAction?: string | null;
  productionConfidence?: number | null;
  productionPositionPct?: number | null;
  /** 容器 test id；子元素 id 以它作前缀，不传则用裸名 */
  testId?: string;
}

/** 取到有限数才算「有值」：NaN / null / undefined 一律当缺席（缺席不塌成 0） */
function finite(v: number | null | undefined): number | null {
  return typeof v === "number" && Number.isFinite(v) ? v : null;
}

/** f7 口径四态（两侧共用同一张码表，避免两套句式漂移） */
function f7SourceKey(source: string | null | undefined): "analystVotes" | "trader" | "absent" | "unknown" {
  if (source === "analyst_votes") { return "analystVotes"; }
  if (source === "trader") { return "trader"; }
  if (source === "absent") { return "absent"; }
  return "unknown";
}

export function F7ReferenceComparison({
  snapshot,
  productionF7Source,
  productionAction,
  productionConfidence,
  productionPositionPct,
  testId,
}: F7ReferenceComparisonProps) {
  const { t } = useTranslation();
  const id = (name: string) => (testId ? `${testId}-${name}` : name);

  if (snapshot == null) {
    return (
      <div data-testid={testId} data-state="not-produced" className="text-sm" style={{ color: "var(--muted)" }}>
        {t("stockAnalysis.f7Reference.notProduced")}
      </div>
    );
  }

  const indAction = snapshot.action ?? null;
  const hasIndAction = !isAbsentDecisionAction(indAction);
  const hasProdAction = !isAbsentDecisionAction(productionAction ?? null);
  const bothActions = hasIndAction && hasProdAction;
  const actionSame = bothActions && parseAction(indAction) === parseAction(productionAction ?? null);

  const refKey = f7SourceKey(snapshot.f7Source);
  const prodKey = f7SourceKey(productionF7Source);
  const sameBasis = refKey === prodKey;
  const collapsed = snapshot.weightsCollapsed === true;

  const indConf = finite(snapshot.confidence);
  const indPos = finite(snapshot.positionPct);
  const prodConf = finite(productionConfidence);
  const prodPos = finite(productionPositionPct);
  // 带符号差（参照侧 − 生产侧）。符号就是方向，不需要在 i18n 里分「高于/低于」两套句式。
  const signed = (
    a: number | null,
    b: number | null,
  ) => (a !== null && b !== null ? `${a - b >= 0 ? "+" : ""}${(a - b).toFixed(1)}` : null);
  const confDelta = signed(indConf, prodConf);
  const posDelta = signed(indPos, prodPos);
  const indPost = finite(snapshot.posterior);
  const f7Pct = finite(snapshot.f7WeightPct);
  const gaps = snapshot.dataGaps ?? [];

  return (
    <div
      data-testid={testId}
      data-state="ok"
      data-f7-source={snapshot.f7Source ?? "unknown"}
      data-production-f7-source={productionF7Source ?? "unknown"}
      data-same-basis={sameBasis ? "true" : "false"}
      data-collapsed={collapsed ? "true" : "false"}
      className="space-y-1 rounded-md p-1.5 text-sm"
      style={{ background: "rgba(217,119,6,0.05)", border: "1px solid rgba(217,119,6,0.14)" }}
    >
      <div data-testid={id("title")} className="font-medium" style={{ color: "#b45309" }}>
        {t("stockAnalysis.f7Reference.title")}
      </div>

      {/* 两侧口径各说一句：哪一侧读交易员观点、哪一侧不读，决定下面的差值能不能被解释成位移 */}
      <div data-testid={id("basis-production")} style={{ color: "var(--color-text-secondary)" }}>
        {t("stockAnalysis.f7Reference.productionLabel")}
        {t(`stockAnalysis.f7Reference.f7.${prodKey}`)}
      </div>
      <div data-testid={id("basis-reference")} style={{ color: "var(--color-text-secondary)" }}>
        {t("stockAnalysis.f7Reference.referenceLabel")}
        {t(`stockAnalysis.f7Reference.f7.${refKey}`)}
      </div>

      {/* ② 同口径 ⇒ 对照本身不成立，必须单独成句（否则「差 0」会被读成「观点无影响」） */}
      {sameBasis && (
        <div data-testid={id("same-basis")} style={{ color: "#b45309" }}>
          {t("stockAnalysis.f7Reference.sameBasis")}
        </div>
      )}

      {collapsed && (
        <div data-testid={id("collapsed")} style={{ color: "#ef4444" }}>
          {t("stockAnalysis.f7Reference.weightsCollapsed")}
        </div>
      )}

      <div className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-0.5">
        <span style={{ color: "var(--muted)" }}>{t("dualView.decision.action")}</span>
        <span>
          {hasIndAction
            ? (
              <span data-testid={id("action")}>
                {t(getActionTKey(parseAction(indAction)))}
              </span>
            )
            : (
              // ④ 参照侧没给方向 ≠ 看空：哨兵值（`数据缺失`）要单列一句，不能套档位标签
              <span data-testid={id("action-absent")} style={{ color: "var(--muted)" }}>
                {t("stockAnalysis.f7Reference.actionAbsent")}
              </span>
            )}
        </span>
        <span style={{ color: "var(--muted)" }}>{t("dualView.decision.confidence")}</span>
        <span data-testid={id("confidence")} className="font-mono">{indConf ?? "—"}</span>
        <span style={{ color: "var(--muted)" }}>{t("dualView.decision.positionPct")}</span>
        <span data-testid={id("position")} className="font-mono">{indPos !== null ? `${indPos}%` : "—"}</span>
        <span style={{ color: "var(--muted)" }}>{t("stockAnalysis.f7Reference.posterior")}</span>
        <span data-testid={id("posterior")} className="font-mono">{indPost !== null ? `${indPost}%` : "—"}</span>
        <span style={{ color: "var(--muted)" }}>{t("stockAnalysis.f7Reference.riskLevel")}</span>
        <span data-testid={id("risk-level")} className="font-mono">
          {snapshot.riskLevel ? t(getRiskTKey(snapshot.riskLevel)) : "—"}
        </span>
        <span style={{ color: "var(--muted)" }}>{t("stockAnalysis.f7Reference.f7Weight")}</span>
        <span data-testid={id("f7-weight")} className="font-mono">
          {f7Pct !== null ? t("stockAnalysis.f7Reference.f7WeightValue", { pct: f7Pct }) : "—"}
        </span>
      </div>

      <div data-testid={id("delta")} style={{ color: "var(--color-text-secondary)" }}>
        <span data-testid={id("action-delta")}>
          {bothActions
            ? (actionSame
              ? t("stockAnalysis.f7Reference.actionSame")
              : t("stockAnalysis.f7Reference.actionDiffer", {
                production: t(getActionTKey(parseAction(productionAction ?? null))),
                reference: t(getActionTKey(parseAction(indAction))),
              }))
            : t("stockAnalysis.f7Reference.actionNotComparable")}
        </span>
        {confDelta !== null && (
          <span data-testid={id("conf-delta")}>
            {" · "}
            {t("stockAnalysis.f7Reference.confDelta", {
              delta: confDelta,
              production: prodConf,
              reference: indConf,
            })}
          </span>
        )}
        {posDelta !== null && (
          <span data-testid={id("pos-delta")}>
            {" · "}
            {t("stockAnalysis.f7Reference.posDelta", {
              delta: posDelta,
              production: prodPos,
              reference: indPos,
            })}
          </span>
        )}
        {confDelta === null && posDelta === null && (
          <span data-testid={id("value-not-comparable")}>
            {" · "}
            {t("stockAnalysis.f7Reference.valueNotComparable")}
          </span>
        )}
      </div>

      {gaps.length > 0
        ? (
          <div data-testid={id("gaps")} style={{ color: "var(--muted)" }}>
            {t("stockAnalysis.f7Reference.gapsPresent", { n: gaps.length, gaps: gaps.join(" · ") })}
          </div>
        )
        : (
          <div data-testid={id("gaps-none")} style={{ color: "var(--muted)" }}>
            {t("stockAnalysis.f7Reference.gapsNone")}
          </div>
        )}

      <div data-testid={id("scale-note")} style={{ color: "var(--muted)" }}>
        {t("stockAnalysis.f7Reference.scaleNote")}
      </div>
    </div>
  );
}
