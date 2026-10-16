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

/** σ 回算结果（对齐后端 `Reconcile`） */
type Reconcile =
  | { state: "consistent"; delta: number; tolerance: number; basis: string }
  | { state: "inconsistent"; delta: number; tolerance: number; basis: string }
  | { state: "noBasis"; reason: string };

/** 全局闭合（对齐后端 `Closure`） */
type Closure =
  | { state: "closed"; recomputed: number; stored: number; tolerance: number }
  | { state: "broken"; recomputed: number; stored: number; tolerance: number }
  | { state: "notComputable"; reason: string };

interface BranchLegAudit {
  factor: string;
  owner: string | null;
  ownerName: string;
  role: string;
  signal: number | null;
  weight: number;
  participates: boolean;
  expectedToParticipate: boolean;
  upstream: UpstreamState;
  absenceDeclared: boolean;
}

interface FactorLegAudit {
  legId: string;
  name: string;
  sigma: number;
  weight: number;
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
  NO_SNAPSHOT_KEY: "noSnapshotKey",
  NO_OWNER: "noOwner",
  UNREGISTERED_FACTOR: "unregisteredFactor",
};

const NO_BASIS_REASON: Record<string, string> = {
  NO_PERSISTED_BASIS: "noPersistedBasis",
  BASIS_MISSING_IN_RECORD: "basisMissingInRecord",
};

const CLOSURE_REASON: Record<string, string> = {
  DECISION_DEGRADED: "decisionDegraded",
  NO_WEIGHT_SUM_OR_FACTORS: "noWeightSumOrFactors",
};

const UPSTREAM_TAG_COLOR: Record<UpstreamState["state"], string> = {
  present: "green",
  declaredAbsent: "default",
  silentAbsent: "orange",
  notChecked: "default",
};

export function EvidenceCitationPanel({ analysisId, visible = true }: Props) {
  const { t } = useTranslation();
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
                  {t("stockAnalysis.evidenceCitation.weight")}={leg.weight.toFixed(3)}
                  {leg.signal !== null
                    ? ` ${t("stockAnalysis.evidenceCitation.signal")}=${leg.signal.toFixed(3)}`
                    : ""}
                </span>
                {!leg.participates && !leg.absenceDeclared && (
                  <Tag color="orange" className="text-[10px] leading-none px-1 py-0 shrink-0">
                    {t("stockAnalysis.evidenceCitation.absenceUndeclared")}
                  </Tag>
                )}
                <Tooltip
                  title={leg.upstream.state === "notChecked"
                    ? t(
                      `stockAnalysis.evidenceCitation.notChecked.${
                        NOT_CHECKED_REASON[leg.upstream.reason] ?? "noSnapshotKey"
                      }`,
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
            <span className="text-green-400/80">
              {t("stockAnalysis.evidenceCitation.sigmaReconciled")}: {report.sigmaReconciledLegs}/
              {report.factorLegsTotal}
            </span>
            <span>
              {t("stockAnalysis.evidenceCitation.participatingLegs")}:{" "}
              {report.participatingFactorLegs}/{report.factorLegsTotal}
            </span>
            <Tooltip
              title={report.closure.state === "notComputable"
                ? t(
                  `stockAnalysis.evidenceCitation.closureReason.${
                    CLOSURE_REASON[report.closure.reason] ?? "noWeightSumOrFactors"
                  }`,
                )
                : undefined}
            >
              <span
                className={report.closure.state === "closed"
                  ? "text-green-400/80"
                  : report.closure.state === "broken"
                  ? "text-red-400/80"
                  : "text-gray-400"}
              >
                {t(`stockAnalysis.evidenceCitation.closure.${report.closure.state}`)}
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
                  σ={leg.sigma} w={leg.weight}
                </span>
                <Tooltip
                  title={leg.reconcile.state === "noBasis"
                    ? t(
                      `stockAnalysis.evidenceCitation.noBasis.${
                        NO_BASIS_REASON[leg.reconcile.reason] ?? "noPersistedBasis"
                      }`,
                    )
                    : leg.reconcile.state !== "inconsistent"
                    ? `${leg.reconcile.basis} Δ=${leg.reconcile.delta.toFixed(4)} ≤ ${
                      leg.reconcile.tolerance.toFixed(4)
                    }`
                    : `${leg.reconcile.basis} Δ=${leg.reconcile.delta.toFixed(4)} > ${
                      leg.reconcile.tolerance.toFixed(4)
                    }`}
                >
                  <Tag
                    color={leg.reconcile.state === "consistent"
                      ? "green"
                      : leg.reconcile.state === "inconsistent"
                      ? "red"
                      : "default"}
                    className="text-[10px] leading-none px-1 py-0 shrink-0"
                  >
                    {t(`stockAnalysis.evidenceCitation.sigma.${leg.reconcile.state}`)}
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
