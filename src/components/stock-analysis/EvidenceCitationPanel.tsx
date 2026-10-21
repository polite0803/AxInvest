/**
 * 决策证据对账审计面板
 *
 * 数据来自 `audit_decision_evidence` 命令。判据在 2026-10-16 换成**决策因子对账**
 * （旧实现是文本相似度：把理由按句拆开后与分析师报告做字符集 Jaccard，实测对
 * 83% 的记录只读出「文案长度」——动机与实证见 `PLAN-evidence-factor-audit.md` §〇）。
 *
 * 面板分两面，**两套计数各自成句、不并排成一个数字**：
 * - 本档分支腿：上游在场 / 显式缺席 / 静默缺席 / 无法核查（快照不携带来源键）
 * - 主档因子腿：σ 可回算 / 不可回算 / 无基准 + 全局闭合
 */

import { invoke } from "@/lib/invoke";
import { Alert, Button, Spin, Tag, Tooltip } from "antd";
import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

/** 上游在场判据（对齐后端 `UpstreamState`，serde 内部标签 `state`） */
type UpstreamState =
  | { state: "present" }
  | { state: "declaredAbsent" }
  | { state: "silentAbsent" }
  | { state: "notChecked"; reason: string };

/**
 * 基准与 σ 的关系（对齐后端 `BasisSource`）。
 *
 * - `independentUpstream`：基准是快照里的独立产出 ⇒ 一致 = 真的核对过
 * - `sameVariableCopy`：基准与 σ 是同一个脚本变量的两条落库副本 ⇒ delta 上限恒等于
 *   两侧舍入界之和，**永远一致**。它只能证明副本未被单边篡改，不能证明信号算对，
 *   因此必须与前者区分展示（2026-10-18 前两者共用一个绿标签）。
 */
type BasisSource = "independentUpstream" | "sameVariableCopy";

/** σ 回算结果（对齐后端 `Reconcile`） */
export type Reconcile =
  | {
    state: "consistent";
    delta: number;
    tolerance: number;
    basis: string;
    source: BasisSource;
  }
  | { state: "inconsistent"; delta: number; tolerance: number; basis: string }
  | { state: "noBasis"; reason: string }
  /**
   * 该腿落库的 `sigma` 字段缺失/类型不对。与 `noBasis` 严格区分：
   * 后者缺的是**基准**，前者缺的是**被检对象**——归因正好相反。
   */
  | { state: "inputMissing"; field: string };

/**
 * 全局闭合（对齐后端 `Closure`）。
 *
 * ⚠ `closed` 只代表 `Σ(σ·w)/weightSum ≟ avgSignal` 这条算术自洽，两侧同源于一次脚本
 * 执行 ⇒ 恒成立。`unverifiedWeightRatio` 交代「参与权重里有多少从未被独立核对」，
 * 没有它，面板会把「63% 权重没核过」显示成纯绿。
 */
export type Closure =
  | {
    state: "closed";
    recomputed: number;
    stored: number;
    tolerance: number;
    unverifiedWeightRatio: number;
  }
  | { state: "broken"; recomputed: number; stored: number; tolerance: number }
  | { state: "notComputable"; reason: string };

interface BranchLegAudit {
  factor: string;
  owner: string | null;
  ownerName: string;
  role: string;
  signal: number | null;
  /** `null` = 字段缺失/类型不对，与「权重真的是 0」可分 */
  weight: number | null;
  participates: boolean;
  expectedToParticipate: boolean;
  upstream: UpstreamState;
  absenceDeclared: boolean;
  /** 裁决层的腿预测力状态；`null/缺省` = 本记录未携带（旧脚本/旧代） */
  icStatus?: string | null;
  /** 该腿是否因「测出无预测力」被裁决层置零 —— 与「没接线/没数」严格区分 */
  icZeroed?: boolean;
}

interface FactorLegAudit {
  legId: string;
  name: string;
  /** `null` = 字段缺失，**不是 0**（后端已不再 `unwrap_or(0.0)`） */
  sigma: number | null;
  weight: number | null;
  owner: string | null;
  participates: boolean;
  reconcile: Reconcile;
}

interface EvidenceAuditReport {
  stockCode: string;
  stockName: string;
  analysisDate: string;
  decisionAction: string;
  decisionConfidence: number;
  /** 本轮决策未产出（后端按 `Unavailable` 哨兵判定）⇒ 无可审计对象 */
  decisionDegraded: boolean;
  tier: string;
  branchLegs: BranchLegAudit[];
  factorLegs: FactorLegAudit[];
  closure: Closure;
  expectedBranchLegs: number;
  participatingBranchLegs: number;
  upstreamPresentLegs: number;
  upstreamNotCheckedLegs: number;
  sigmaReconciledLegs: number;
  participatingFactorLegs: number;
  factorLegsTotal: number;
}

interface Props {
  analysisId: string;
  visible?: boolean;
}

const NOT_CHECKED_REASON: Record<string, string> = {
  // 逐档评分节点的产出**不落主快照**（阶段 0 实测 0/49）⇒ 该腿判不了，如实说「不适用」
  TIER_SCORE_NOT_IN_SNAPSHOT: "tierScoreNotInSnapshot",
  // 来源通道没有登记落库键（映射表缺项）—— 有门在挡（通道齐备门），出现即表被写漏
  NO_CHANNEL_MAPPING: "noChannelMapping",
  NO_OWNER: "noOwner",
  UNREGISTERED_FACTOR: "unregisteredFactor",
  INJECTED_VAR_HAS_NO_KEY_TABLE: "injectedVarNoKeyTable",
};

/** 原因码不在上表时的兜底 —— 不许猜原因，只如实说「后端给了个未登记的原因码」。 */
const UNKNOWN_REASON = "unknownReason";

/**
 * 后端原因码 → i18n 文案键。
 *
 * 抽成**具名导出**是为了能被直测：原因句只出现在 antd `Tooltip` 的 title 里，而
 * rc-tooltip 的正文是**懒渲染**的（不 hover 不进 DOM）⇒ 走渲染断言只能断到状态标签，
 * 断不到「哪一种不适用」这句话。测这个纯映射，才能锁住「每个码有专属文案、未知码走兜底」。
 */
export function notCheckedReasonKey(reason: string): string {
  return NOT_CHECKED_REASON[reason] ?? UNKNOWN_REASON;
}

const NO_BASIS_REASON: Record<string, string> = {
  NO_PERSISTED_BASIS: "noPersistedBasis",
  BASIS_MISSING_IN_RECORD: "basisMissingInRecord",
};

const CLOSURE_REASON: Record<string, string> = {
  DECISION_DEGRADED: "decisionDegraded",
  NO_WEIGHT_SUM_OR_FACTORS: "noWeightSumOrFactors",
  MISSING_FACTOR_INPUT: "missingFactorInput",
};

/**
 * 数值渲染。`null` **不得**显示成 0 —— 0 是「算出来是零」，`null` 是「根本没给」，
 * 二者在面板上同形正是这条链路上一版假绿的成因之一。
 */
export function fmtNum(v: number | null, digits = 3): string {
  return v === null ? "—" : v.toFixed(digits);
}

const UPSTREAM_TAG_COLOR: Record<UpstreamState["state"], string> = {
  present: "green",
  declaredAbsent: "default",
  silentAbsent: "orange",
  notChecked: "default",
};

/**
 * σ 标签的文案键。
 *
 * `consistent` 与 `sameSourceCopy` **必须分成两句话**：后者的基准与 σ 是同一脚本变量的
 * 两条落库副本，delta 上限恒等于两侧舍入界之和 ⇒ 它永远一致，不构成「信号被核对过」的
 * 证据。合并成一句绿标签就是 2026-10-18 截图里 `6/12` 那个假读数的来源。
 */
export function sigmaLabelKey(
  r: Reconcile,
): "consistent" | "sameSourceCopy" | "inconsistent" | "noBasis" | "inputMissing" {
  if (r.state === "consistent") {
    return r.source === "sameVariableCopy" ? "sameSourceCopy" : "consistent";
  }
  if (r.state === "inputMissing") {
    return "inputMissing";
  }
  return r.state === "inconsistent" ? "inconsistent" : "noBasis";
}

/** 同源副本不得用与「独立核对」相同的绿色；σ 缺失比「不一致」更靠上游，单独一色。 */
export function sigmaTagColor(r: Reconcile): string {
  if (r.state === "consistent") {
    return r.source === "sameVariableCopy" ? "gold" : "green";
  }
  if (r.state === "inputMissing") {
    return "volcano";
  }
  return r.state === "inconsistent" ? "red" : "default";
}

/** σ 腿的悬浮说明：先交代基准关系，再给读数。 */
export function sigmaTooltip(r: Reconcile, t: (key: string) => string): string {
  const prefix = "stockAnalysis.evidenceCitation";
  if (r.state === "inputMissing") {
    return t(`${prefix}.sigma.inputMissingHint`);
  }
  if (r.state === "noBasis") {
    return t(`${prefix}.noBasis.${NO_BASIS_REASON[r.reason] ?? "noPersistedBasis"}`);
  }
  const relation = r.state === "consistent" && r.source === "sameVariableCopy"
    ? t(`${prefix}.sigma.sameSourceCopyHint`)
    : "";
  const delta = `Δ=${r.delta.toFixed(4)} ${r.delta <= r.tolerance ? "≤" : ">"} ${r.tolerance.toFixed(4)}`;
  return [r.basis, delta, relation].filter(Boolean).join("\n");
}

/** 闭合覆盖率：参与权重里「经过独立核对」的比例；`notComputable` / `broken` 无覆盖率可言。 */
export function closureCoverage(c: Closure): number | null {
  if (c.state !== "closed") { return null; }
  // 字段缺失（旧后端 / 键名错配）时 `1 - undefined` 是 NaN，绝不能渲染成 `NaN%`；
  // 拿不到覆盖率就只报「闭合」，不伪造任何比例。
  return Number.isFinite(c.unverifiedWeightRatio) ? 1 - c.unverifiedWeightRatio : null;
}

/**
 * 闭合标签。`closed` 必须**带上覆盖率**——它只证明算术自洽，同源两侧恒成立；
 * 只说「闭合」会把「63% 权重没核过」读成「证据链已对账」。
 */
export function closureLabel(c: Closure, t: (key: string) => string): string {
  const prefix = "stockAnalysis.evidenceCitation";
  const base = t(`${prefix}.closure.${c.state === "notComputable" ? "notComputable" : c.state}`);
  const coverage = closureCoverage(c);
  if (coverage === null) {
    return base;
  }
  return `${base} · ${t(`${prefix}.closure.coverage`)} ${Math.round(coverage * 100)}%`;
}

/** 闭合的悬浮读数：`broken` 时也必须给出差值，否则红了也不知道差多少。 */
export function closureTooltip(c: Closure, t: (key: string) => string): string {
  const prefix = "stockAnalysis.evidenceCitation";
  if (c.state === "notComputable") {
    return t(`${prefix}.closureReason.${CLOSURE_REASON[c.reason] ?? "noWeightSumOrFactors"}`);
  }
  const delta = Math.abs(c.recomputed - c.stored);
  const base = `Σ(σ·w)/W=${c.recomputed.toFixed(4)} stored=${c.stored.toFixed(4)} `
    + `Δ=${delta.toFixed(4)} ${delta <= c.tolerance ? "≤" : ">"} tol=${c.tolerance.toFixed(4)}`;
  return c.state === "closed"
    ? `${base} unverifiedWeight=${(c.unverifiedWeightRatio * 100).toFixed(0)}%`
    : base;
}

export function EvidenceCitationPanel({ analysisId, visible = true }: Props) {
  const { t } = useTranslation();
  // 辅助函数只要「键 → 文案」这一层能力：`TFunction` 的返回类型含 object，
  // 直接传给 `(key: string) => string` 不兼容，故在此显式收窄一次。
  const tk = (key: string) => String(t(key));
  const [report, setReport] = useState<EvidenceAuditReport | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // 依赖只留 analysisId：`t` 每轮渲染都是新引用，放进 deps 会让本 callback 每轮重建
  // ⇒ effect 自激（实测一次挂载把这条 IPC 刷了 38 次，回归用例锁在 1 次）。
  const loadAudit = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const result = await invoke<EvidenceAuditReport>("audit_decision_evidence", {
        analysisId,
      });
      setReport(result);
    } catch (e: unknown) {
      setError(typeof e === "string" ? e : e instanceof Error ? e.message : "");
    } finally {
      setLoading(false);
    }
  }, [analysisId]);

  useEffect(() => {
    if (visible && analysisId) {
      loadAudit();
    }
  }, [visible, analysisId, loadAudit]);

  if (!visible) { return null; }

  if (loading) {
    return (
      <div className="flex justify-center py-8">
        <Spin description={t("stockAnalysis.evidenceCitation.loading")} />
      </div>
    );
  }

  // 用 `!== null` 而非真值判断：错误已发生但消息为空串时也必须停在错误态。
  if (error !== null) {
    return (
      <Alert
        type="error"
        title={t("stockAnalysis.evidenceCitation.error")}
        description={error || t("stockAnalysis.evidenceCitation.extractFailed")}
        showIcon
      />
    );
  }

  // 决策未产出：本轮没有可审计对象。必须**先于**任何空态分支，且不得渲染任何计数。
  if (report?.decisionDegraded) {
    return (
      <Alert
        type="warning"
        showIcon
        title={t("stockAnalysis.evidenceCitation.notApplicable")}
        description={t("stockAnalysis.evidenceCitation.notApplicableDetail", {
          action: report.decisionAction,
        })}
      />
    );
  }

  if (!report || (report.branchLegs.length === 0 && report.factorLegs.length === 0)) {
    return (
      <div className="text-gray-400 text-sm text-center py-6">
        {t("stockAnalysis.evidenceCitation.noData")}
      </div>
    );
  }

  // 覆盖率只在 `closed` 时存在。低于 50% ⇒ 大部分参与权重从未被独立核对，
  // 「闭合」不得染成与「核对通过」同一种绿。
  const closureCov = closureCoverage(report.closure);

  return (
    <div className="space-y-3">
      <div className="flex items-center justify-between">
        <div>
          <span className="text-sm font-medium text-gray-200">
            {t("stockAnalysis.evidenceCitation.title")}
          </span>
          <span className="text-xs text-gray-500 ml-2">
            {report.stockCode} · {report.analysisDate} · {report.tier}
          </span>
        </div>
        <Button size="small" onClick={loadAudit}>
          {t("common.refresh")}
        </Button>
      </div>

      {/* ── B 面：本档分支腿 ── */}
      {report.branchLegs.length > 0 && (
        <div className="bg-gray-800/40 rounded p-2 space-y-2">
          <div className="text-xs font-medium text-gray-300">
            {t("stockAnalysis.evidenceCitation.branchSection")}
          </div>
          <div className="flex flex-wrap gap-x-4 gap-y-1 text-[11px] text-gray-400">
            <span>
              {t("stockAnalysis.evidenceCitation.expectedLegs")}: {report.expectedBranchLegs}
            </span>
            <span>
              {t("stockAnalysis.evidenceCitation.participatingLegs")}: {report.participatingBranchLegs}
            </span>
            <span className="text-green-400/80">
              {t("stockAnalysis.evidenceCitation.upstreamPresentCount")}: {report.upstreamPresentLegs}
            </span>
            <span>
              {t("stockAnalysis.evidenceCitation.upstreamNotCheckedCount")}: {report.upstreamNotCheckedLegs}
            </span>
          </div>
          <div className="space-y-1">
            {report.branchLegs.map((leg) => (
              <div
                key={leg.factor}
                className="flex items-baseline gap-2 text-xs"
                style={{ flexWrap: "wrap" }}
              >
                <span className="font-mono text-gray-300 shrink-0">{leg.factor}</span>
                <Tag className="text-[10px] leading-none px-1 py-0 shrink-0">{leg.ownerName}</Tag>
                <span className="text-gray-500 font-mono text-[11px]">
                  {t("stockAnalysis.evidenceCitation.weight")}={fmtNum(leg.weight)}
                  {leg.signal !== null
                    ? ` ${t("stockAnalysis.evidenceCitation.signal")}=${leg.signal.toFixed(3)}`
                    : ""}
                </span>
                {
                  /* 应参与却未计入有两种完全不同的原因：裁决层测出「无预测力」后主动
                    置零（有交代），与其它未计入（没接线/没数）。前者必须单独成标，
                    否则「测过了被裁掉」会被读成链路故障。 */
                }
                {leg.expectedToParticipate && !leg.participates && (
                  leg.icZeroed
                    ? (
                      <Tooltip
                        title={leg.icStatus
                          ? `${t("stockAnalysis.evidenceCitation.icZeroedHint")} · ${leg.icStatus}`
                          : t("stockAnalysis.evidenceCitation.icZeroedHint")}
                      >
                        <Tag color="default" className="text-[10px] leading-none px-1 py-0 shrink-0">
                          {t("stockAnalysis.evidenceCitation.icZeroed")}
                        </Tag>
                      </Tooltip>
                    )
                    : (
                      <Tag color="gold" className="text-[10px] leading-none px-1 py-0 shrink-0">
                        {t("stockAnalysis.evidenceCitation.expectedButIdle")}
                      </Tag>
                    )
                )}
                {!leg.participates && !leg.absenceDeclared && !leg.icZeroed && (
                  <Tag color="orange" className="text-[10px] leading-none px-1 py-0 shrink-0">
                    {t("stockAnalysis.evidenceCitation.absenceUndeclared")}
                  </Tag>
                )}
                <Tooltip
                  title={leg.upstream.state === "notChecked"
                    ? t(
                      `stockAnalysis.evidenceCitation.notChecked.${notCheckedReasonKey(leg.upstream.reason)}`,
                    )
                    : undefined}
                >
                  <Tag
                    color={UPSTREAM_TAG_COLOR[leg.upstream.state]}
                    className="text-[10px] leading-none px-1 py-0 shrink-0"
                  >
                    {t(`stockAnalysis.evidenceCitation.upstream.${leg.upstream.state}`)}
                  </Tag>
                </Tooltip>
              </div>
            ))}
          </div>
        </div>
      )}

      {/* ── A 面：主档因子闭合对账 ── */}
      {report.factorLegs.length > 0 && (
        <div className="bg-gray-800/40 rounded p-2 space-y-2">
          <div className="text-xs font-medium text-gray-300">
            {t("stockAnalysis.evidenceCitation.factorSection")}
          </div>
          <div className="flex flex-wrap gap-x-4 gap-y-1 text-[11px] text-gray-400">
            {
              /* 分子 = 参与且经独立上游核对；分母 = 参与腿数。两侧同口径，
                读者不会把「6/12 与 9/12」误读成「9 条参与里 6 条可核」。 */
            }
            <span className="text-green-400/80">
              {t("stockAnalysis.evidenceCitation.sigmaReconciled")}: {report.sigmaReconciledLegs}/
              {report.participatingFactorLegs}
            </span>
            <span>
              {t("stockAnalysis.evidenceCitation.participatingLegs")}:{" "}
              {report.participatingFactorLegs}/{report.factorLegsTotal}
            </span>
            <Tooltip title={closureTooltip(report.closure, tk)}>
              <span
                className={report.closure.state === "closed"
                  ? closureCov !== null && closureCov >= 0.5
                    ? "text-green-400/80"
                    : "text-amber-400/80"
                  : report.closure.state === "broken"
                  ? "text-red-400/80"
                  : "text-gray-400"}
              >
                {closureLabel(report.closure, tk)}
              </span>
            </Tooltip>
          </div>
          <div className="space-y-1">
            {report.factorLegs.map((leg) => (
              <div
                key={leg.legId}
                className="flex items-baseline gap-2 text-xs"
                style={{ flexWrap: "wrap" }}
              >
                <span className="font-mono text-gray-400 shrink-0">{leg.legId}</span>
                <span className="text-gray-300 shrink-0">{leg.name}</span>
                <Tag className="text-[10px] leading-none px-1 py-0 shrink-0">
                  {leg.owner ?? t("stockAnalysis.evidenceCitation.metaLeg")}
                </Tag>
                <span className="text-gray-500 font-mono text-[11px]">
                  σ={fmtNum(leg.sigma)} w={fmtNum(leg.weight)}
                </span>
                <Tooltip title={sigmaTooltip(leg.reconcile, tk)}>
                  <Tag
                    color={sigmaTagColor(leg.reconcile)}
                    className="text-[10px] leading-none px-1 py-0 shrink-0"
                  >
                    {t(`stockAnalysis.evidenceCitation.sigma.${sigmaLabelKey(leg.reconcile)}`)}
                  </Tag>
                </Tooltip>
              </div>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}
