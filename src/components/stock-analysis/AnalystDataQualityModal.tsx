// i18n-exempt: 业务逻辑判断字符串（节点类型/状态枚举键名），非 UI 展示文本
import { diagStatusToSeverity, parseDataQualityReport, resolveAnalystDiagnosis } from "@/lib/dataQualityDiagnosis";
import { invoke } from "@/lib/invoke";
import { useStockAnalysisStore } from "@/stores";
import type { DataQualityDiagItem } from "@/types";
import { CheckCircleFilled, CloseCircleFilled, ExclamationCircleFilled, ThunderboltFilled } from "@ant-design/icons";
import { Button, Col, Modal, Progress, Row, Table, Tag, Tooltip, Typography } from "antd";
import type { ColumnsType } from "antd/es/table";
import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";

const { Text } = Typography;

// ── 严重度图标 ──────────────────────────────────────────────
const STATUS_ICON: Record<string, React.ReactNode> = {
  good: <CheckCircleFilled style={{ color: "#52c41a" }} />,
  warning: <ExclamationCircleFilled style={{ color: "#faad14" }} />,
  issue: <CloseCircleFilled style={{ color: "#f5222d" }} />,
};

// ── 节点类型检测 ──────────────────────────────────────────────
type NodeType = "analyst" | "debate" | "decision" | "tool" | "valuation" | "risk" | "other";

/** 根据 expertId/nodeId 推断节点类型 */
function detectNodeType(nodeId: string): NodeType {
  if (nodeId.startsWith("a-")) { return "analyst"; }
  if (nodeId.startsWith("bull-") || nodeId.startsWith("bear-")) { return "debate"; }
  if (nodeId.includes("decision") || nodeId.includes("manager")) { return "decision"; }
  if (nodeId.startsWith("t-") || nodeId.startsWith("u-")) { return "tool"; }
  if (nodeId.includes("valuation")) { return "valuation"; }
  if (nodeId.includes("risk")) { return "risk"; }
  return "other";
}

/** 获取节点类型的 i18n key */
function getNodeTypeName(nodeType: NodeType): string {
  const names: Record<NodeType, string> = {
    analyst: "stockAnalysis.analystReport.nodeTypeAnalyst",
    debate: "stockAnalysis.analystReport.nodeTypeDebate",
    decision: "stockAnalysis.analystReport.nodeTypeDecision",
    tool: "stockAnalysis.analystReport.nodeTypeTool",
    valuation: "stockAnalysis.analystReport.nodeTypeValuation",
    risk: "stockAnalysis.analystReport.nodeTypeRisk",
    other: "stockAnalysis.analystReport.nodeTypeOther",
  };
  return names[nodeType];
}

/** 后端诊断状态 → 文案 key */
function diagStatusKey(status: DataQualityDiagItem["status"]): string {
  switch (status) {
    case "normal":
      return "stockAnalysis.analystReport.dqStatusNormal";
    case "low":
      return "stockAnalysis.analystReport.dqStatusLow";
    case "missing":
      return "stockAnalysis.analystReport.dqStatusMissing";
    default:
      return "stockAnalysis.analystReport.dqStatusUntrusted";
  }
}

const GRADE_COLOR: Record<string, string> = {
  A: "#52c41a",
  B: "#73d13d",
  C: "#faad14",
  D: "#fa8c16",
  F: "#f5222d",
};

// ── 诊断明细行 ──────────────────────────────────────────────
interface DiagRow {
  field: string;
  severity: "good" | "warning" | "issue";
  detail: string;
}

// ── 组件 ──────────────────────────────────────────────────
interface Props {
  name: string;
  expertId: string;
  open: boolean;
  onClose: () => void;
  /** 可选：股票代码，用于关联分析 */
  stockCode?: string;
  /** 可选：执行 ID，用于关联工作流执行 */
  executionId?: string;
}

/**
 * 数据质量面板。
 *
 * 2026-09-14 重构：本组件原带一套独立的数据质量算法（analyzeDataQuality），
 * 只解析单个节点的 JSON 结构、不读报告正文，与工作流里 data-quality.rhai 的
 * 口径完全不同 —— 同一份报告在弹窗里是 A、在决策链里被记为该节点降级并把
 * 全局拉到 C。现改为纯展示：等级、分数、逐节点诊断全部取自 data-quality 节点
 * 的权威输出（store.dataQualitySummary），前端不再自行计算任何等级。
 */
export function AnalystDataQualityModal({
  name,
  expertId,
  open,
  onClose,
  stockCode = "",
  executionId = "",
}: Props) {
  const { t } = useTranslation();

  // data-quality 节点输出（JSON 字符串），由 store 从 workflow results / blackboard 快照提取
  const dataQualitySummary = useStockAnalysisStore((s) => s.dataQualitySummary) ?? "";
  const report = useMemo(() => parseDataQualityReport(dataQualitySummary), [dataQualitySummary]);
  const diag = useMemo(() => resolveAnalystDiagnosis(expertId, report, name), [expertId, report, name]);

  // 检测节点类型
  const nodeType = detectNodeType(expertId);
  const nodeTypeName = t(getNodeTypeName(nodeType));
  const nodeTypeUiName = t(
    `stockAnalysis.analystReport.nodeType${nodeType.charAt(0).toUpperCase() + nodeType.slice(1)}`,
  );

  const rows = useMemo<DiagRow[]>(() => {
    if (!diag) { return []; }
    const hits = diag.placeholder_hits ?? 0;
    const sev = diagStatusToSeverity(diag.status);
    return [
      {
        field: t("stockAnalysis.analystReport.dqFieldNodeStatus"),
        severity: sev,
        detail: `${t(diagStatusKey(diag.status))} · confidence=${diag.confidence}`,
      },
      {
        field: t("stockAnalysis.analystReport.dqFieldPlaceholder"),
        severity: hits > 0 ? "warning" : "good",
        detail: hits > 0
          ? t("stockAnalysis.analystReport.dqPlaceholderCount", { count: hits })
          : t("stockAnalysis.analystReport.dqPlaceholderNone"),
      },
      {
        field: t("stockAnalysis.analystReport.dqFieldExpected"),
        severity: "good",
        detail: diag.expected_data || "—",
      },
      {
        field: t("stockAnalysis.analystReport.dqFieldGapReason"),
        severity: diag.gap_reason ? sev : "good",
        detail: diag.gap_reason || t("stockAnalysis.analystReport.dqGapReasonNone"),
      },
    ];
  }, [diag, t]);

  // 面板打开时上报诊断结果给后端，供节点自我进化消费。
  // 2026-09-14: 上报内容由「前端自算的 grade/score」改为「data-quality 节点的权威输出 +
  //   本节点诊断」—— 前者会让自我进化基于第二套口径学习，与决策链实际消费的等级不一致。
  useEffect(() => {
    if (!open || !report || !diag) { return; }

    const issueCount = diag.status === "missing" || diag.status === "untrusted" ? 1 : 0;
    const warningCount = diag.status === "low" ? 1 : 0;
    const goodCount = diag.status === "normal" ? 1 : 0;

    const qualityMetrics = {
      dqi_grade: report.grade,
      dqi_score: report.score,
      report_quality_score: report.report_quality_score ?? null,
      tool_credibility_score: report.tool_credibility_score ?? null,
      factor_completeness_pct: report.factor_completeness_pct ?? null,
      node_status: diag.status,
      node_confidence: diag.confidence,
      node_placeholder_hits: diag.placeholder_hits ?? 0,
    };

    invoke("save_node_feedback", {
      request: {
        nodeType,
        nodeId: expertId,
        reportId: `report-${Date.now()}`,
        stockCode,
        executionId,
        qualityScore: Math.round(report.score),
        grade: report.grade,
        issueCount,
        warningCount,
        goodCount,
        checksJson: JSON.stringify([{
          field: "node_status",
          status: diag.status,
          confidence: diag.confidence,
          placeholder_hits: diag.placeholder_hits ?? 0,
          gap_reason: diag.gap_reason,
        }]),
        qualityMetricsJson: JSON.stringify(qualityMetrics),
      },
    }).catch((err) => {
      console.warn(`Failed to save ${nodeTypeName} feedback for self-evolution:`, err);
    });
  }, [open, report, diag, nodeType, expertId, stockCode, executionId, nodeTypeName]);

  // 节点自我进化状态
  const [evolving, setEvolving] = useState(false);
  const [evolutionStatus, setEvolutionStatus] = useState<string | null>(null);
  const [evolutionSuggestions, setEvolutionSuggestions] = useState<string[]>([]);

  const handleEvolve = async () => {
    setEvolving(true);
    setEvolutionStatus(null);
    setEvolutionSuggestions([]);
    try {
      const status = await invoke<{
        node_type: string;
        node_id: string;
        total_feedbacks: number;
        status: string;
        suggestions: string[];
      }>("evolve_node_command", {
        request: {
          nodeType,
          nodeId: expertId,
        },
      });
      setEvolutionStatus(status.status);
      setEvolutionSuggestions(status.suggestions || []);
    } catch (err) {
      console.error(`Failed to evolve ${nodeTypeName}:`, err);
      setEvolutionStatus("error");
    } finally {
      setEvolving(false);
    }
  };

  const columns: ColumnsType<DiagRow> = [
    {
      title: "",
      dataIndex: "severity",
      key: "icon",
      width: 32,
      render: (s: string) => STATUS_ICON[s] ?? null,
    },
    {
      title: t("stockAnalysis.analystReport.dataQualityFieldCompleteness"),
      dataIndex: "field",
      key: "field",
      width: 180,
      render: (val: string) => <code>{val}</code>,
    },
    {
      title: t("stockAnalysis.analystReport.dataQualityOverall"),
      dataIndex: "detail",
      key: "detail",
      render: (val: string) => <Text style={{ fontSize: 12 }}>{val}</Text>,
    },
  ];

  return (
    <Modal
      title={
        <span>
          {name} — {t("stockAnalysis.analystReport.dataQuality")}
        </span>
      }
      open={open}
      onCancel={onClose}
      footer={
        <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
          {/* 左侧：进化状态 */}
          <div style={{ flex: 1 }}>
            {evolutionStatus === "healthy" && (
              <Tag color="success">
                ✓ {t("stockAnalysis.analystReport.evolutionHealthy", { nodeName: nodeTypeUiName })}
              </Tag>
            )}
            {evolutionStatus === "needs_attention" && (
              <Tag color="warning">
                ⚠ {t("stockAnalysis.analystReport.evolutionNeedsAttention", { nodeName: nodeTypeUiName })}
              </Tag>
            )}
            {evolutionStatus === "collecting_data" && (
              <Tag color="blue">
                {t("stockAnalysis.analystReport.evolutionCollectingData", { nodeName: nodeTypeUiName })}
              </Tag>
            )}
            {evolutionStatus === "no_data" && (
              <Tag color="default">
                {t("stockAnalysis.analystReport.evolutionNoData", { nodeName: nodeTypeUiName })}
              </Tag>
            )}
            {evolutionStatus === "error" && (
              <Tag color="error">{t("stockAnalysis.analystReport.evolutionFailed", { nodeName: nodeTypeUiName })}</Tag>
            )}
          </div>
          {/* 右侧：操作按钮 */}
          <div style={{ display: "flex", gap: 8 }}>
            <Button onClick={onClose}>{t("stockAnalysis.analystReport.close")}</Button>
            <Button
              type="primary"
              icon={<ThunderboltFilled />}
              loading={evolving}
              onClick={handleEvolve}
              disabled={!report}
            >
              {t("stockAnalysis.analystReport.evolutionTrigger", { nodeName: nodeTypeUiName })}
            </Button>
          </div>
        </div>
      }
      width={680}
      style={{ top: 40 }}
      styles={{ body: { maxHeight: "70vh", overflow: "auto" } }}
    >
      {!report
        ? (
          <div style={{ padding: 24, textAlign: "center" }}>
            <Text type="secondary">{t("stockAnalysis.analystReport.dqNoNodeDiagnosis")}</Text>
          </div>
        )
        : (
          <>
            {/* 全局等级（由 data-quality 节点统一产出）*/}
            <Row gutter={24} style={{ marginBottom: 16 }}>
              <Col span={8} style={{ textAlign: "center" }}>
                <Progress
                  type="circle"
                  percent={report.score}
                  size={80}
                  strokeColor={GRADE_COLOR[report.grade] ?? "#888780"}
                  format={(pct) => `${Math.round(pct ?? 0)}`}
                />
                <div style={{ marginTop: 4 }}>
                  <Text style={{ fontSize: 12, color: "var(--muted)" }}>
                    {t("stockAnalysis.analystReport.dataQualityScore")}
                  </Text>
                </div>
              </Col>
              <Col span={8} style={{ textAlign: "center" }}>
                <div
                  style={{
                    fontSize: 48,
                    fontWeight: 700,
                    color: GRADE_COLOR[report.grade] ?? "#888780",
                    lineHeight: 1,
                    marginTop: 16,
                  }}
                >
                  {report.grade}
                </div>
                <div style={{ marginTop: 4 }}>
                  <Text style={{ fontSize: 12, color: "var(--muted)" }}>
                    {t("stockAnalysis.analystReport.dataQualityOverall")}
                  </Text>
                </div>
              </Col>
              <Col span={8} style={{ textAlign: "center", paddingTop: 20 }}>
                <div style={{ display: "flex", justifyContent: "center", gap: 12 }}>
                  <Tooltip title={t("stockAnalysis.analystReport.dataQualityGood")}>
                    <Tag color="success">{report.good_count}</Tag>
                  </Tooltip>
                  <Tooltip title={t("stockAnalysis.analystReport.dataQualityWarning")}>
                    <Tag color="warning">{report.degraded_count ?? 0}</Tag>
                  </Tooltip>
                  <Tooltip title={t("stockAnalysis.analystReport.dataQualityIssue")}>
                    <Tag color="error">{report.gap_count}</Tag>
                  </Tooltip>
                </div>
                <div style={{ marginTop: 4 }}>
                  <Text style={{ fontSize: 12, color: "var(--muted)" }}>
                    {t("stockAnalysis.analystReport.dataQualityCheckCount", { count: report.total_analysts })}
                  </Text>
                </div>
              </Col>
            </Row>

            {/* 三维分解 */}
            <div
              style={{
                marginBottom: 16,
                padding: "8px 12px",
                background: "var(--ant-color-fill-quaternary)",
                borderRadius: 8,
              }}
            >
              <Text style={{ fontSize: 12 }}>
                {t("stockAnalysis.analystReport.dqDimensionReport")}{" "}
                <strong>{report.report_quality_score ?? "—"}</strong> × 0.35 ·{" "}
                {t("stockAnalysis.analystReport.dqDimensionTool")}{" "}
                <strong>{report.tool_credibility_score ?? "—"}</strong> × 0.35 ·{" "}
                {t("stockAnalysis.analystReport.dqDimensionFactor")}{" "}
                <strong>{report.factor_completeness_pct ?? "—"}</strong>% × 0.30
              </Text>
              <div style={{ marginTop: 4 }}>
                <Text type="secondary" style={{ fontSize: 11 }}>
                  {t("stockAnalysis.analystReport.dqScoreSource")}
                </Text>
              </div>
            </div>

            {/* 本节点诊断 */}
            <div style={{ marginBottom: 8 }}>
              <Text strong style={{ fontSize: 13 }}>
                {t("stockAnalysis.analystReport.dqNodeDiagnosis")}
              </Text>
            </div>
            {rows.length > 0
              ? (
                <Table
                  dataSource={rows}
                  columns={columns}
                  rowKey={(r) => r.field}
                  pagination={false}
                  size="small"
                  bordered
                  style={{ fontSize: 12 }}
                  onHeaderRow={() => ({ style: { fontSize: 12 } })}
                />
              )
              : (
                <Text type="secondary" style={{ fontSize: 12 }}>
                  {t("stockAnalysis.analystReport.dqNoNodeDiagnosis")}
                </Text>
              )}
            {report.direction_conflict === true && (
              <div style={{ marginTop: 8 }}>
                <Text type="warning" style={{ fontSize: 12 }}>
                  {t("stockAnalysis.analystReport.dqDirectionConflict", {
                    bull: report.bull_dir_count ?? 0,
                    bear: report.bear_dir_count ?? 0,
                  })}
                </Text>
              </div>
            )}

            {/* 自我进化建议 */}
            {evolutionSuggestions.length > 0 && (
              <div style={{ marginTop: 16, padding: 12, background: "var(--ant-color-info-bg)", borderRadius: 8 }}>
                <Text strong>{t("stockAnalysis.analystReport.evolutionSuggestion", { nodeName: nodeTypeUiName })}</Text>
                <ul style={{ margin: "8px 0 0 0", paddingLeft: 20 }}>
                  {evolutionSuggestions.map((s, i) => (
                    <li key={i}>
                      <Text type="secondary" style={{ fontSize: 12 }}>{s}</Text>
                    </li>
                  ))}
                </ul>
              </div>
            )}
          </>
        )}
    </Modal>
  );
}
