// i18n-exempt: 配置映射表/业务数据字符串，非用户可见 UI 文案
import {
  isRiskAggregateKey,
  parseRiskAssessment,
  parseRiskLevel,
  RISK_VIEW_KEYS,
  RISK_VIEW_ROWS,
  type RiskAssessmentView,
  type RiskLevelView,
  riskStanceLabelKey,
} from "@/lib/risk-assessment";
import { useSettingsStore, useStockAnalysisStore } from "@/stores";
import { DownloadOutlined, ExpandOutlined } from "@ant-design/icons";
import { Button, Card, Modal, Spin, Tag } from "antd";
import type { TFunction } from "i18next";
import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { HorizonDecisionStrip } from "./HorizonDecisionStrip";
import { HorizonScopeNotice } from "./HorizonScopeNotice";
import { ReportMarkdown } from "./ReportMarkdown";

/** 风险类型 → 颜色（OKLch 值与 index.css --sa-* 同步） */
const RISK_COLORS: Record<string, string> = {
  "risk-agg": "oklch(55% 0.20 28)",
  "risk-con": "oklch(55% 0.18 150)",
  "risk-neu": "oklch(55% 0.16 250)",
  "research-mgr": "oklch(60% 0.18 85)",
  "comprehensive": "oklch(60% 0.16 290)",
  "risk-aggregated": "oklch(55% 0.20 28)",
  "risk-level": "oklch(55% 0.18 45)",
  "risk-convergence": "oklch(55% 0.16 200)",
};

/** 风险类型 → i18n key（key 与 riskAssessments 实际节点 ID 对齐） */
const RISK_LABEL_KEYS: Record<string, string> = {
  "risk-agg": "stockAnalysis.risk.aggressive",
  "risk-con": "stockAnalysis.risk.conservative",
  "risk-neu": "stockAnalysis.risk.neutral",
  "research-mgr": "stockAnalysis.risk.researchManager",
  "comprehensive": "stockAnalysis.risk.comprehensive",
  "risk-aggregated": "stockAnalysis.workflow.riskAggregation",
  "agg-risk": "stockAnalysis.workflow.riskAggregation",
  "risk-convergence": "stockAnalysis.workflow.riskConvergence",
  "risk-level": "stockAnalysis.riskLevel",
};

const AXIS_LABEL_KEYS: Record<string, string> = {
  drawdownDepth: "stockAnalysis.riskMatrix.axisDrawdownDepth",
  fundamentals: "stockAnalysis.riskMatrix.axisFundamentals",
  sharpe: "stockAnalysis.riskMatrix.axisSharpe",
  volatility: "stockAnalysis.riskMatrix.axisVolatility",
};

const AXIS_VALUE_KEYS: Record<string, string> = {
  absent: "stockAnalysis.riskMatrix.axisAbsent",
  latestReport: "stockAnalysis.riskMatrix.axisValueLatestReport",
  global60: "stockAnalysis.riskMatrix.axisValueGlobal60",
};

const METRIC_META: Record<string, { key: string; percent?: boolean; digits?: number }> = {
  volatility_pct: { key: "stockAnalysis.riskMatrix.metricVolatility", percent: true },
  sharpe: { key: "stockAnalysis.riskMatrix.metricSharpe", digits: 3 },
  max_drawdown_pct: { key: "stockAnalysis.riskMatrix.metricMaxDrawdown", percent: true },
  drawdown_depth: { key: "stockAnalysis.riskMatrix.metricDrawdownDepth", percent: true },
  debt_ratio_pct: { key: "stockAnalysis.riskMatrix.metricDebtRatio", percent: true },
  gross_margin_pct: { key: "stockAnalysis.riskMatrix.metricGrossMargin", percent: true },
  revenue_growth_yoy_pct: { key: "stockAnalysis.riskMatrix.metricRevenueGrowth", percent: true },
  roe_ttm_pct: { key: "stockAnalysis.riskMatrix.metricRoe", percent: true },
  window_days: { key: "stockAnalysis.riskMatrix.metricWindowDays" },
};
const METRIC_ORDER = Object.keys(METRIC_META);

interface RiskViewRow {
  key: string;
  labelKey: string;
  present: boolean;
  view: RiskAssessmentView | null;
}

function stanceLabel(stance: string | null, t: TFunction): string {
  if (!stance) { return "—"; }
  const key = riskStanceLabelKey(stance);
  return key ? t(key) : stance;
}

function riskCategoryColor(category: string | null): string {
  if (!category) { return "default"; }
  if (category.includes("极高")) { return "red"; }
  if (category.includes("中")) { return "orange"; }
  if (category.includes("高")) { return "red"; }
  if (category.includes("低")) { return "green"; }
  return "default";
}

function formatMetricValue(key: string, value: number | null, t: TFunction): string {
  if (value === null) { return t("stockAnalysis.riskMatrix.axisAbsent"); }
  const meta = METRIC_META[key];
  if (!meta) { return String(value); }
  if (meta.percent) { return `${value}%`; }
  if (meta.digits !== undefined) { return value.toFixed(meta.digits); }
  return String(value);
}

function axisLabel(key: string, t: TFunction): string {
  const labelKey = AXIS_LABEL_KEYS[key];
  return labelKey ? t(labelKey) : key;
}

function axisValue(value: string, t: TFunction): string {
  const valueKey = AXIS_VALUE_KEYS[value];
  return valueKey ? t(valueKey) : value;
}

function riskEntryLabel(key: string, t: TFunction): string {
  const labelKey = RISK_LABEL_KEYS[key];
  return labelKey ? t(labelKey) : key;
}

/** 把风险评估各分组序列化为可分享的 Markdown 文档 */
function buildRiskMarkdown(
  data: {
    views: RiskViewRow[];
    riskLevel: RiskLevelView | null;
    convergence: RiskAssessmentView | null;
    research: RiskAssessmentView | null;
    otherEntries: Array<[string, string]>;
    stockCode?: string;
    stockName?: string;
  },
  t: TFunction,
): string {
  const now = new Date().toISOString().replace("T", " ").substring(0, 19);
  const title = data.stockName && data.stockCode
    ? `${data.stockName} (${data.stockCode}) — ${t("stockAnalysis.riskMatrix.exportTitle")}`
    : t("stockAnalysis.riskMatrix.exportTitle");
  const lines: string[] = [
    `# ${title}`,
    "",
    `> ${t("stockAnalysis.riskMatrix.exportTime")}：${now}`,
    "",
    `## ${t("stockAnalysis.riskMatrix.viewsTitle")}`,
    "",
    `| ${t("stockAnalysis.riskMatrix.verdictStance")} | ${t("stockAnalysis.riskMatrix.verdictPosition")} | ${
      t("stockAnalysis.riskMatrix.verdictConfidence")
    } |`,
    "| --- | --- | --- |",
  ];
  for (const row of data.views) {
    const label = t(row.labelKey);
    if (!row.present || !row.view) {
      lines.push(`| ${label} | ${t("stockAnalysis.riskMatrix.noVote")} | — |`);
      continue;
    }
    const { stance, positionPct, confidence } = row.view.verdict;
    lines.push(
      `| ${label} · ${stanceLabel(stance, t)} | ${positionPct === null ? "—" : `${positionPct}%`} | ${
        confidence === null ? "—" : `${Math.round(confidence)}`
      } |`,
    );
  }
  lines.push("");

  lines.push(`## ${t("stockAnalysis.riskMatrix.riskLevelTitle")}`, "");
  if (data.riskLevel) {
    if (data.riskLevel.category) { lines.push(`- **${t("stockAnalysis.riskLevel")}**：${data.riskLevel.category}`); }
    if (data.riskLevel.matchedRules.length > 0) {
      lines.push(`- **${t("stockAnalysis.riskMatrix.matchedRules")}**：${data.riskLevel.matchedRules.join("；")}`);
    }
    const axes = Object.entries(data.riskLevel.axisScope);
    if (axes.length > 0) {
      lines.push(
        `- **${t("stockAnalysis.riskMatrix.axisGaps")}**：${
          axes.map(([key, value]) => `${axisLabel(key, t)}=${axisValue(value, t)}`).join("；")
        }`,
      );
    }
    const metrics = Object.entries(data.riskLevel.metrics);
    if (metrics.length > 0) {
      lines.push(
        `- **${t("stockAnalysis.riskMatrix.metricsTitle")}**：${
          metrics.map(([key, value]) =>
            `${METRIC_META[key] ? t(METRIC_META[key].key) : key}=${formatMetricValue(key, value, t)}`
          )
            .join("；")
        }`,
      );
    }
  } else {
    lines.push(`> ${t("stockAnalysis.riskMatrix.noRiskLevel")}`);
  }
  lines.push("");

  lines.push(`## ${t("stockAnalysis.riskMatrix.convergenceTitle")}`, "");
  lines.push(
    data.convergence?.report
      ? data.convergence.report
      : `> ${t("stockAnalysis.riskMatrix.noConvergence")}`,
  );
  lines.push("");

  lines.push(`## ${t("stockAnalysis.riskMatrix.researchManagerTitle")}`, "");
  lines.push(
    data.research?.report
      ? data.research.report
      : `> ${t("stockAnalysis.riskMatrix.noResearchManager")}`,
  );
  lines.push("");

  if (data.otherEntries.length > 0) {
    lines.push(`## ${t("stockAnalysis.riskMatrix.otherAssessments")}`, "");
    for (const [key, raw] of data.otherEntries) {
      lines.push(`### ${riskEntryLabel(key, t)}`, "");
      lines.push(parseRiskAssessment(raw).report || `> ${t("stockAnalysis.noRiskData")}`, "");
    }
  }

  lines.push(`## ${t("stockAnalysis.riskMatrix.detailAssessment")}`, "");
  for (const row of data.views) {
    if (!row.present || !row.view?.report) { continue; }
    lines.push(`### ${t(row.labelKey)}`, "", row.view.report, "");
  }

  lines.push(
    `${t("stockAnalysis.riskMatrix.exportDisclaimer", { appName: t("app.name") })}`,
  );
  return lines.join("\n");
}

function RiskBody({
  views,
  riskLevel,
  convergence,
  research,
  otherEntries,
  isDark,
  dense,
  onOpen,
}: {
  views: RiskViewRow[];
  riskLevel: RiskLevelView | null;
  convergence: RiskAssessmentView | null;
  research: RiskAssessmentView | null;
  otherEntries: Array<[string, string]>;
  isDark: boolean;
  dense: boolean;
  onOpen: (key: string) => void;
}) {
  const { t } = useTranslation();
  const [researchOpen, setResearchOpen] = useState(false);
  const [othersOpen, setOthersOpen] = useState(false);

  const metricEntries = riskLevel
    ? Object.entries(riskLevel.metrics).sort(([a], [b]) => {
      const ia = METRIC_ORDER.indexOf(a);
      const ib = METRIC_ORDER.indexOf(b);
      return (ia === -1 ? METRIC_ORDER.length : ia) - (ib === -1 ? METRIC_ORDER.length : ib);
    })
    : [];

  return (
    <div className={dense ? "space-y-2" : "space-y-4"}>
      {/* 收敛结论置顶：三视角的裁决先给结论，再给三票 */}
      <div className="p-2 rounded" style={{ background: "var(--surface)" }}>
        <div className="flex items-center gap-1 mb-1">
          <Tag color={RISK_COLORS["risk-convergence"]} style={{ marginRight: 0 }}>
            {t("stockAnalysis.riskMatrix.convergenceTitle")}
          </Tag>
        </div>
        {convergence?.report
          ? (
            <div
              className="sa-markdown-content text-xs leading-relaxed"
              style={{ maxHeight: dense ? 220 : 360, overflow: "auto" }}
            >
              <ReportMarkdown content={convergence.report} isDark={isDark} />
            </div>
          )
          : (
            <div className="text-xs" style={{ color: "var(--muted)" }}>
              {t("stockAnalysis.riskMatrix.noConvergence")}
            </div>
          )}
      </div>

      {/* 三视角：固定激进 / 保守 / 中性三格，缺票明说 */}
      <div>
        <div className="text-xs font-semibold mb-1" style={{ color: "var(--muted)" }}>
          {t("stockAnalysis.riskMatrix.viewsTitle")}
        </div>
        <div className={`grid grid-cols-1 ${dense ? "sm:grid-cols-3" : "md:grid-cols-3"} gap-2`}>
          {views.map((row) => {
            const color = RISK_COLORS[row.key] ?? "oklch(55% 0.16 250)";
            return (
              <div
                key={row.key}
                className="p-2 rounded"
                style={{ background: "var(--surface)", borderTop: `2px solid ${color}` }}
              >
                <div className="flex items-center justify-between mb-1">
                  <Tag color={color} style={{ marginRight: 0 }}>{t(row.labelKey)}</Tag>
                  {row.present && (
                    <Button
                      type="text"
                      size="small"
                      icon={<ExpandOutlined />}
                      style={{ padding: 0, width: 18, height: 18, fontSize: 10 }}
                      onClick={() => onOpen(row.key)}
                      title={t("stockAnalysis.riskMatrix.expandDetail")}
                    />
                  )}
                </div>
                {row.present && row.view
                  ? (
                    <>
                      <div className={dense ? "text-xs space-y-0.5" : "text-sm space-y-1"}>
                        <div>
                          <span style={{ color: "var(--muted)" }}>{t("stockAnalysis.riskMatrix.verdictStance")}：</span>
                          {stanceLabel(row.view.verdict.stance, t)}
                        </div>
                        <div>
                          <span style={{ color: "var(--muted)" }}>
                            {t("stockAnalysis.riskMatrix.verdictPosition")}：
                          </span>
                          {row.view.verdict.positionPct === null ? "—" : `${row.view.verdict.positionPct}%`}
                        </div>
                        <div>
                          <span style={{ color: "var(--muted)" }}>
                            {t("stockAnalysis.riskMatrix.verdictConfidence")}：
                          </span>
                          {row.view.verdict.confidence === null ? "—" : Math.round(row.view.verdict.confidence)}
                        </div>
                      </div>
                      <div
                        className="sa-markdown-content text-xs leading-relaxed mt-1"
                        style={{ maxHeight: 180, overflow: "auto" }}
                      >
                        {row.view.report
                          ? <ReportMarkdown content={row.view.report} isDark={isDark} />
                          : <span style={{ color: "var(--muted)" }}>{t("stockAnalysis.noRiskData")}</span>}
                      </div>
                    </>
                  )
                  : (
                    <div className="text-xs" style={{ color: "var(--muted)" }}>
                      {t("stockAnalysis.riskMatrix.noVote")}
                    </div>
                  )}
              </div>
            );
          })}
        </div>
      </div>

      {/* 规则风险档：category + 命中规则 + 口径缺席 + 关键指标 */}
      <div className="p-2 rounded" style={{ background: "var(--surface)" }}>
        <div className="flex items-center gap-1 flex-wrap mb-1">
          <Tag color={RISK_COLORS["risk-level"]} style={{ marginRight: 0 }}>
            {t("stockAnalysis.riskMatrix.riskLevelTitle")}
          </Tag>
          {riskLevel?.category && <Tag color={riskCategoryColor(riskLevel.category)}>{riskLevel.category}</Tag>}
          {riskLevel?.degraded && <Tag color="orange">{t("stockAnalysis.riskMatrix.degraded")}</Tag>}
        </div>
        {riskLevel
          ? (
            <div className="text-xs space-y-1">
              {riskLevel.matchedRules.length > 0 && (
                <div>
                  <span style={{ color: "var(--muted)" }}>{t("stockAnalysis.riskMatrix.matchedRules")}：</span>
                  {riskLevel.matchedRules.join("；")}
                </div>
              )}
              {Object.keys(riskLevel.axisScope).length > 0 && (
                <div>
                  <span style={{ color: "var(--muted)" }}>{t("stockAnalysis.riskMatrix.axisGaps")}：</span>
                  {Object.entries(riskLevel.axisScope)
                    .map(([key, value]) => `${axisLabel(key, t)}=${axisValue(value, t)}`)
                    .join("；")}
                </div>
              )}
              {metricEntries.length > 0 && (
                <div>
                  <div style={{ color: "var(--muted)" }}>{t("stockAnalysis.riskMatrix.metricsTitle")}：</div>
                  <div className="grid grid-cols-2 sm:grid-cols-4 gap-x-3 gap-y-0.5 mt-0.5">
                    {metricEntries.map(([key, value]) => (
                      <div key={key}>
                        <span style={{ color: "var(--muted)" }}>
                          {METRIC_META[key] ? t(METRIC_META[key].key) : key}：
                        </span>
                        {formatMetricValue(key, value, t)}
                      </div>
                    ))}
                  </div>
                </div>
              )}
              {riskLevel.warnings.length > 0 && (
                <div style={{ color: "var(--sa-amber)" }}>{riskLevel.warnings.join("；")}</div>
              )}
            </div>
          )
          : <div className="text-xs" style={{ color: "var(--muted)" }}>{t("stockAnalysis.riskMatrix.noRiskLevel")}
          </div>}
      </div>

      {/* 研究经理：默认折叠（长文），展开看正文 */}
      <div className="p-2 rounded" style={{ background: "var(--surface)" }}>
        <button
          type="button"
          className="flex items-center gap-1 text-xs font-semibold"
          style={{ color: "var(--muted)", background: "none", border: 0, padding: 0, cursor: "pointer" }}
          onClick={() => setResearchOpen((v) => !v)}
        >
          <Tag color={RISK_COLORS["research-mgr"]} style={{ marginRight: 0 }}>
            {t("stockAnalysis.riskMatrix.researchManagerTitle")}
          </Tag>
          {researchOpen ? t("stockAnalysis.riskMatrix.collapse") : t("stockAnalysis.riskMatrix.expand")}
        </button>
        {researchOpen && (
          <div className="sa-markdown-content text-xs leading-relaxed mt-1">
            {research?.report
              ? <ReportMarkdown content={research.report} isDark={isDark} />
              : <span style={{ color: "var(--muted)" }}>{t("stockAnalysis.riskMatrix.noResearchManager")}</span>}
          </div>
        )}
      </div>

      {/* 其他风险产物：不静默丢，默认折叠 */}
      {otherEntries.length > 0 && (
        <div className="p-2 rounded" style={{ background: "var(--surface)" }}>
          <button
            type="button"
            className="flex items-center gap-1 text-xs font-semibold"
            style={{ color: "var(--muted)", background: "none", border: 0, padding: 0, cursor: "pointer" }}
            onClick={() => setOthersOpen((v) => !v)}
          >
            {t("stockAnalysis.riskMatrix.otherAssessments")}（{otherEntries.length}）
            {othersOpen ? t("stockAnalysis.riskMatrix.collapse") : t("stockAnalysis.riskMatrix.expand")}
          </button>
          {othersOpen && (
            <div className="space-y-2 mt-1">
              {otherEntries.map(([key, raw]) => {
                const view = parseRiskAssessment(raw);
                return (
                  <div key={key}>
                    <Tag color={RISK_COLORS[key] ?? "default"}>{riskEntryLabel(key, t)}</Tag>
                    <div className="sa-markdown-content text-xs leading-relaxed">
                      {view.report
                        ? <ReportMarkdown content={view.report} isDark={isDark} />
                        : <span style={{ color: "var(--muted)" }}>{t("stockAnalysis.noRiskData")}</span>}
                    </div>
                  </div>
                );
              })}
            </div>
          )}
        </div>
      )}
    </div>
  );
}

export function RiskMatrix() {
  const themeMode = useSettingsStore((s) => s.settings.themeMode);
  const isDark = themeMode === "dark"
    || (themeMode === "system" && window.matchMedia("(prefers-color-scheme: dark)").matches);
  const riskAssessments = useStockAnalysisStore((s) => s.riskAssessments);
  const streamingPreviews = useStockAnalysisStore((s) => s.streamingPreviews);
  const workflowStatus = useStockAnalysisStore((s) => s.status);
  const stockCode = useStockAnalysisStore((s) => s.stockCode);
  const stockName = useStockAnalysisStore((s) => s.stockName);
  // 四周期风险档随逐档决策带回（`decisionsByHorizon[].riskCategory`），不是 riskAssessments 产物
  const decision = useStockAnalysisStore((s) => s.decision);
  const { t } = useTranslation();
  const [expanded, setExpanded] = useState(false);
  const [selectedCard, setSelectedCard] = useState<string | null>(null);
  const [exporting, setExporting] = useState(false);

  const views = useMemo<RiskViewRow[]>(
    () =>
      RISK_VIEW_ROWS.map(({ key, labelKey }) => {
        const raw = riskAssessments[key];
        const present = typeof raw === "string" && raw.trim().length > 0;
        return {
          key,
          labelKey,
          present,
          view: present ? parseRiskAssessment(raw) : null,
        };
      }),
    [riskAssessments],
  );

  const riskLevelRaw = riskAssessments["risk-level"];
  const convergenceRaw = riskAssessments["risk-convergence"];
  const researchRaw = riskAssessments["research-mgr"];
  // 解析引用要稳定：otherEntries 的 useMemo 依赖它，每次渲染新建对象会让缓存失效
  const { riskLevel, convergence, research } = useMemo(
    () => ({
      riskLevel: typeof riskLevelRaw === "string" ? parseRiskLevel(riskLevelRaw) : null,
      convergence: typeof convergenceRaw === "string" ? parseRiskAssessment(convergenceRaw) : null,
      research: typeof researchRaw === "string" ? parseRiskAssessment(researchRaw) : null,
    }),
    [riskLevelRaw, convergenceRaw, researchRaw],
  );

  const otherEntries = useMemo(() => {
    const rows: Array<[string, string]> = [];
    for (const [key, raw] of Object.entries(riskAssessments)) {
      if (typeof raw !== "string") { continue; }
      if (RISK_VIEW_KEYS.includes(key)) { continue; }
      if (key === "risk-convergence" || key === "research-mgr") { continue; }
      if (isRiskAggregateKey(key)) { continue; }
      // risk-level 认不出结构化字段时按普通产物保留，不静默丢
      if (key === "risk-level" && riskLevel) { continue; }
      rows.push([key, raw]);
    }
    return rows;
  }, [riskAssessments, riskLevel]);

  const hasAnyData = views.some((v) => v.present)
    || riskLevel !== null
    || convergence !== null
    || research !== null
    || otherEntries.length > 0;

  const isWorkflowRunning = workflowStatus === "running" || workflowStatus === "loading";

  // 风险阶段流式预览：仅风险链路节点，且尚未落地的（完成后由主区接管）
  const runningRiskPreviews = useMemo(() => {
    if (!isWorkflowRunning) { return []; }
    return Object.entries(streamingPreviews).filter(([nodeId]) =>
      (nodeId.startsWith("risk-") || nodeId === "research-mgr")
      && riskAssessments[nodeId] === undefined
    );
  }, [streamingPreviews, riskAssessments, isWorkflowRunning]);

  // 四周期条及其「哪根轴按档」声明与 riskAssessments 无关 ⇒ 空数据时也要渲染
  const horizonHeader = (
    <>
      <HorizonScopeNotice scope="risk" />
      <HorizonDecisionStrip
        decisions={decision?.decisionsByHorizon}
        mode="risk"
        testId="risk-horizon-strip"
      />
    </>
  );

  if (!hasAnyData) {
    if (isWorkflowRunning) {
      return (
        <>
          {horizonHeader}
          <Card
            size="small"
            title={t("stockAnalysis.riskAssessment")}
            styles={{ body: { padding: 24 } }}
          >
            <div className="flex flex-col items-center justify-center gap-3 py-8">
              <Spin size="large" />
              <div className="text-sm" style={{ color: "var(--muted)" }}>
                {t("stockAnalysis.riskMatrix.loading")}
              </div>
              {runningRiskPreviews.map(([nodeId, text]) => {
                const color = RISK_COLORS[nodeId] ?? "oklch(55% 0.16 250)";
                return (
                  <div key={`streaming-${nodeId}`} className="w-full px-2">
                    <div className="flex items-center gap-1">
                      <Tag color={color}>{riskEntryLabel(nodeId, t)}</Tag>
                      <Tag color="processing" style={{ fontSize: 10 }}>
                        {t("stockAnalysis.workflow.running")}
                      </Tag>
                    </div>
                    <pre
                      className="text-xs leading-relaxed whitespace-pre-wrap mt-1 px-2 py-1 rounded"
                      style={{ maxHeight: 160, overflow: "auto", color: "var(--muted)", margin: 0 }}
                    >
                    {text.length > 600 ? text.slice(-600) : text}
                    </pre>
                  </div>
                );
              })}
            </div>
          </Card>
        </>
      );
    }
    return decision?.decisionsByHorizon ? horizonHeader : null;
  }

  const handleExportRiskMarkdown = async () => {
    if (exporting) { return; }
    setExporting(true);
    try {
      const md = buildRiskMarkdown(
        { views, riskLevel, convergence, research, otherEntries, stockCode, stockName },
        t,
      );
      const { exportAsMarkdown } = await import("@/lib/exportChat");
      const safeName = (stockName && stockCode)
        ? `${stockName}-${stockCode}-${t("stockAnalysis.riskMatrix.exportFilename")}`
        : t("stockAnalysis.riskMatrix.exportFilename");
      await exportAsMarkdown(md as unknown as import("@/types").Message[], safeName);
    } finally {
      setExporting(false);
    }
  };

  const body = (
    <RiskBody
      views={views}
      riskLevel={riskLevel}
      convergence={convergence}
      research={research}
      otherEntries={otherEntries}
      isDark={isDark}
      dense={!expanded}
      onOpen={setSelectedCard}
    />
  );

  const selectedRaw = selectedCard ? riskAssessments[selectedCard] : undefined;
  const selectedView = typeof selectedRaw === "string" ? parseRiskAssessment(selectedRaw) : null;

  return (
    <>
      {/* 「哪根轴按档」声明 + 四周期逐档风险档（`riskCategory`） */}
      {horizonHeader}
      <Card
        size="small"
        title={t("stockAnalysis.riskAssessment")}
        extra={
          <Button
            type="text"
            size="small"
            icon={<ExpandOutlined />}
            onClick={() => setExpanded(true)}
          />
        }
        styles={{ body: { padding: 8 } }}
      >
        {body}
        {runningRiskPreviews.length > 0 && (
          <div className="space-y-1.5 mt-2">
            {runningRiskPreviews.map(([nodeId, text]) => {
              const color = RISK_COLORS[nodeId] ?? "oklch(55% 0.16 250)";
              return (
                <div key={`streaming-${nodeId}`} className="p-1.5 rounded" style={{ background: "var(--surface)" }}>
                  <div className="text-sm font-medium mb-0.5 flex items-center gap-1">
                    <Tag color={color} style={{ marginRight: 4 }}>{riskEntryLabel(nodeId, t)}</Tag>
                    <Tag color="processing" style={{ fontSize: 10, marginRight: 0 }}>
                      {t("stockAnalysis.workflow.running")}
                    </Tag>
                  </div>
                  <pre
                    className="text-xs leading-relaxed whitespace-pre-wrap"
                    style={{ maxHeight: 160, overflow: "auto", color: "var(--muted)", margin: 0 }}
                  >
                    {text.length > 600 ? text.slice(-600) : text}
                  </pre>
                </div>
              );
            })}
          </div>
        )}
      </Card>

      <Modal
        title={t("stockAnalysis.riskAssessment")}
        open={expanded}
        onCancel={() => setExpanded(false)}
        footer={[
          <Button
            key="export"
            type="primary"
            icon={<DownloadOutlined />}
            loading={exporting}
            onClick={handleExportRiskMarkdown}
          >
            {t("common.exportMarkdown")}
          </Button>,
        ]}
        width="80vw"
        style={{ top: 20 }}
        styles={{ body: { maxHeight: "80vh", overflow: "auto" } }}
      >
        {body}
      </Modal>

      {/* 单个风险产物全文 */}
      <Modal
        title={selectedCard
          ? (
            <div className="flex items-center gap-2">
              <Tag color={RISK_COLORS[selectedCard] ?? "default"}>{riskEntryLabel(selectedCard, t)}</Tag>
            </div>
          )
          : ""}
        open={!!selectedCard}
        onCancel={() => setSelectedCard(null)}
        footer={null}
        width="70vw"
        style={{ top: 40 }}
        styles={{ body: { maxHeight: "75vh", overflow: "auto", padding: 24 } }}
      >
        {selectedView && (
          <div className="sa-markdown-content leading-relaxed text-base">
            {selectedView.report
              ? <ReportMarkdown content={selectedView.report} isDark={isDark} />
              : <span style={{ color: "var(--muted)" }}>{t("stockAnalysis.noRiskData")}</span>}
          </div>
        )}
      </Modal>
    </>
  );
}
