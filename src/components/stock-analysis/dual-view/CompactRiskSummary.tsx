// i18n-exempt: 业务逻辑/日志字符串，非 UI 展示文本
/**
 * CompactRiskSummary — RiskMatrix 在 chat 中的紧凑版本。
 *
 * 与主面板同源（`@/lib/risk-assessment`）：固定激进/保守/中性三行，各自展示
 * verdict 的立场/建议仓位/置信度，再给规则档 category。
 *
 * 这里**不再有第二套风险分**：此前它用关键词词频另算一套分数（主面板用 confidence），
 * 同一份数据在 chat 与主面板读数不一致；也没有过滤 `risk-aggregated`，
 * 聚合容器会参与平均并挤掉一个真实视角。两处都随本轮收口。
 */
import { parseRiskAssessment, parseRiskLevel, RISK_VIEW_ROWS, riskStanceLabelKey } from "@/lib/risk-assessment";
import { useStockAnalysisStore } from "@/stores/feature/stockAnalysisStore";
import { useMemo } from "react";
import { useTranslation } from "react-i18next";

interface CompactRiskSummaryProps {
  data: Record<string, string> | unknown;
}

function normalizeMap(data: CompactRiskSummaryProps["data"]): Record<string, string> {
  if (data && typeof data === "object" && !Array.isArray(data)) {
    const out: Record<string, string> = {};
    for (const [key, value] of Object.entries(data as Record<string, unknown>)) {
      if (typeof value === "string") { out[key] = value; }
    }
    return out;
  }
  return {};
}

export function CompactRiskSummary({ data }: CompactRiskSummaryProps) {
  const { t } = useTranslation();
  const untrustedNodes = useStockAnalysisStore((s) => s.untrustedNodes);
  const assessments = useMemo(() => normalizeMap(data), [data]);
  const untrustedRiskNodes = useMemo(
    () => Object.keys(untrustedNodes).filter((key) => key.startsWith("risk-") || key === "research-mgr"),
    [untrustedNodes],
  );

  const rows = useMemo(
    () =>
      RISK_VIEW_ROWS.map(({ key, labelKey }) => {
        const raw = assessments[key];
        const present = typeof raw === "string" && raw.trim().length > 0;
        return {
          key,
          label: t(labelKey),
          present,
          verdict: present ? parseRiskAssessment(raw).verdict : null,
        };
      }),
    [assessments, t],
  );

  const riskLevelRaw = assessments["risk-level"];
  const riskLevel = riskLevelRaw ? parseRiskLevel(riskLevelRaw) : null;
  // 聚合容器不是第四个视角：只有它时不渲染三视角摘要（否则会显示三行「未产出」）。
  // 收敛/研究经理单独存在时仍渲染 —— 三视角确实缺票，要如实说「未产出」而不是「无数据」。
  const hasViewData = rows.some((row) => row.present)
    || riskLevel !== null
    || typeof assessments["risk-convergence"] === "string"
    || typeof assessments["research-mgr"] === "string";

  if (!hasViewData) {
    return (
      <div className="text-[12px] italic" style={{ color: "var(--muted)" }}>
        {t("stockAnalysis.noRiskData")}
      </div>
    );
  }

  return (
    <div className="space-y-1 text-[12px]">
      {untrustedRiskNodes.length > 0 && (
        <div
          className="rounded px-2 py-1 text-[11px] font-medium"
          style={{
            background: "var(--red-bg)",
            color: "var(--red)",
            border: "1px solid var(--red)",
          }}
        >
          ⚠️ {t("stockAnalysis.dataAlert", { nodes: untrustedRiskNodes.join(", ") })}
        </div>
      )}
      {rows.map((row) => {
        const stanceKey = row.verdict ? riskStanceLabelKey(row.verdict.stance) : null;
        return (
          <div key={row.key} className="flex items-baseline gap-1.5">
            <span style={{ color: "var(--muted)", minWidth: 48 }}>{row.label}</span>
            {row.present && row.verdict
              ? (
                <span style={{ color: "var(--color-text-secondary)" }}>
                  {stanceKey ? t(stanceKey) : row.verdict.stance ?? "—"}
                  {" · "}
                  {row.verdict.positionPct === null ? "—" : `${row.verdict.positionPct}%`}
                  {" · "}
                  {row.verdict.confidence === null ? "—" : Math.round(row.verdict.confidence)}
                </span>
              )
              : <span style={{ color: "var(--muted)" }}>{t("stockAnalysis.riskMatrix.noVote")}</span>}
          </div>
        );
      })}
      <div className="flex items-baseline gap-1.5">
        <span style={{ color: "var(--muted)", minWidth: 48 }}>
          {t("stockAnalysis.riskMatrix.riskLevelTitle")}
        </span>
        <span style={{ color: "var(--color-text-secondary)" }}>
          {riskLevel?.category ?? t("stockAnalysis.riskMatrix.noRiskLevel")}
        </span>
      </div>
    </div>
  );
}
