import {
  getCatalystDirectionColor,
  getCatalystDirectionTKey,
  getCatalystTimelineTKey,
  getChecklistCategoryTKey,
  getDashboardActionColor,
  getDashboardActionTKey,
  getDashboardSeverityColor,
  getDashboardSeverityTKey,
  getDashboardTrendColor,
  getDashboardTrendTKey,
} from "@/lib/stock-analysis-utils";
import { useSettingsStore } from "@/stores";
import type { Catalyst, ChecklistItem, DashboardReport, RiskAlert } from "@/types";
import { AlertOutlined, BulbOutlined, CheckCircleOutlined, SafetyCertificateOutlined } from "@ant-design/icons";
import { Alert, Card, Checkbox, Progress, Space, Tag, Typography } from "antd";
import { useTranslation } from "react-i18next";
import { ReportMarkdown } from "./ReportMarkdown";

const { Text, Title } = Typography;

/** 格式化可选数字 */
function fmtNum(v?: number | null): string {
  if (v === null || v === undefined) {
    return "—";
  }
  return v.toFixed(2);
}

/** 风险警报区块 */
function RiskAlertsSection({ alerts }: { alerts: RiskAlert[] }) {
  const { t } = useTranslation();
  const themeMode = useSettingsStore((s) => s.settings.themeMode);
  const isDark = themeMode === "dark"
    || (themeMode === "system" && window.matchMedia("(prefers-color-scheme: dark)").matches);
  if (alerts.length === 0) {
    return null;
  }
  return (
    <Card
      size="small"
      title={
        <Space>
          <AlertOutlined />
          {t("stockAnalysis.dashboard.riskAlerts")}
        </Space>
      }
      style={{ marginTop: 12 }}
    >
      <Space orientation="vertical" style={{ width: "100%" }}>
        {alerts.map((alert, idx) => (
          <div key={idx}>
            <Tag color={getDashboardSeverityColor(alert.severity)}>
              {t(getDashboardSeverityTKey(alert.severity) ?? alert.severity)}
            </Tag>
            {alert.source && <Tag>{alert.source}</Tag>}
            <ReportMarkdown content={alert.description ?? ""} isDark={isDark} />
          </div>
        ))}
      </Space>
    </Card>
  );
}

/** 催化因素区块 */
function CatalystsSection({ catalysts }: { catalysts: Catalyst[] }) {
  const { t } = useTranslation();
  const themeMode = useSettingsStore((s) => s.settings.themeMode);
  const isDark = themeMode === "dark"
    || (themeMode === "system" && window.matchMedia("(prefers-color-scheme: dark)").matches);
  if (catalysts.length === 0) {
    return null;
  }
  return (
    <Card
      size="small"
      title={
        <Space>
          <BulbOutlined />
          {t("stockAnalysis.dashboard.catalysts")}
        </Space>
      }
      style={{ marginTop: 12 }}
    >
      <Space orientation="vertical" style={{ width: "100%" }}>
        {catalysts.map((cat, idx) => (
          <div key={idx}>
            <Tag color={getCatalystDirectionColor(cat.direction)}>
              {t(getCatalystDirectionTKey(cat.direction) ?? cat.direction)}
            </Tag>
            {cat.timeline && <Tag>{t(getCatalystTimelineTKey(cat.timeline) ?? cat.timeline)}</Tag>}
            <ReportMarkdown content={cat.description ?? ""} isDark={isDark} />
            {cat.confidenceScore !== null && cat.confidenceScore !== undefined && (
              <Text type="secondary" style={{ marginLeft: 8 }}>
                {t("stockAnalysis.dashboard.confidence")}: {cat.confidenceScore.toFixed(0)}%
              </Text>
            )}
          </div>
        ))}
      </Space>
    </Card>
  );
}

/** 操作检查清单区块 */
function ChecklistSection({ items }: { items: ChecklistItem[] }) {
  const { t } = useTranslation();
  const themeMode = useSettingsStore((s) => s.settings.themeMode);
  const isDark = themeMode === "dark"
    || (themeMode === "system" && window.matchMedia("(prefers-color-scheme: dark)").matches);
  if (items.length === 0) {
    return null;
  }
  return (
    <Card
      size="small"
      title={
        <Space>
          <CheckCircleOutlined />
          {t("stockAnalysis.dashboard.checklist")}
        </Space>
      }
      style={{ marginTop: 12 }}
    >
      <Space orientation="vertical" style={{ width: "100%" }}>
        {items.map((item, idx) => (
          <Checkbox key={idx} checked={item.checked} disabled>
            <Tag>{t(getChecklistCategoryTKey(item.category) ?? item.category)}</Tag>
            <ReportMarkdown content={item.description ?? ""} isDark={isDark} />
          </Checkbox>
        ))}
      </Space>
    </Card>
  );
}

/** 决策仪表盘预览组件 */
export function DashboardReportPreview({ report }: { report: DashboardReport }) {
  const { t } = useTranslation();
  const themeMode = useSettingsStore((s) => s.settings.themeMode);
  const isDark = themeMode === "dark"
    || (themeMode === "system" && window.matchMedia("(prefers-color-scheme: dark)").matches);

  const scoreColor = report.score >= 60 ? "#52c41a" : report.score >= 30 ? "#faad14" : "#f5222d";

  // ── 「交易目标价」是否含信息量（2026-09-13）──
  // LLM trader 在持有/观望档会把 targetPrice 抄成等于现价（603466 实证：13.27 == 13.27），
  // 这不含任何方向信息。此处与后端 `portfolio-mgr.rhai` 的 R-204 判定**同一容差口径**（相对 0.5%），
  // 等值时显示「未设」而非具体数字 —— 否则用户会把「目标价 = 现价」与下方估值区间读成自相矛盾。
  const tp = report.targetPrice;
  const cp = report.currentPrice;
  const targetPriceMeaningful = tp != null && tp !== undefined
    && (cp == null || cp === undefined || cp <= 0 || Math.abs(tp - cp) / cp >= 0.005);
  // 内在价值区间（估值语义，来自 t-valuation 的 DCF 三档）——与交易价位严格区分
  const intrinsicRange = report.intrinsicValueLow != null && report.intrinsicValueHigh != null
    ? `${fmtNum(report.intrinsicValueLow)} - ${fmtNum(report.intrinsicValueHigh)}`
    : report.intrinsicValueMid != null
    ? fmtNum(report.intrinsicValueMid)
    : null;

  return (
    <div style={{ padding: 16 }}>
      {/* 标题 */}
      <Title level={4}>
        {report.stockName}({report.stockCode}) {t("stockAnalysis.dashboard.title")}
      </Title>
      <Space size="middle" style={{ marginBottom: 12 }}>
        <Text type="secondary">📅 {report.analysisDate}</Text>
        <Text type="secondary">
          🤖 {report.llmModel ?? "—"}
        </Text>
        {!report.integrityPassed && (
          <Alert
            type="warning"
            showIcon
            title={t("stockAnalysis.dashboard.integrityWarning")}
            style={{ padding: "2px 8px" }}
          />
        )}
      </Space>

      {/* 1. 核心结论 */}
      <Card
        size="small"
        title={
          <Space>
            <SafetyCertificateOutlined />
            {t("stockAnalysis.dashboard.coreConclusion")}
          </Space>
        }
      >
        <Space wrap style={{ marginBottom: 8 }}>
          <Tag color={getDashboardActionColor(report.action)} style={{ fontSize: 14, padding: "2px 8px" }}>
            {t(getDashboardActionTKey(report.action))}
          </Tag>
          <Tag color={getDashboardTrendColor(report.trend)}>
            {t(getDashboardTrendTKey(report.trend) ?? report.trend)}
          </Tag>
          <Text>
            📊 {t("stockAnalysis.dashboard.score")}:{" "}
            <Text strong style={{ color: scoreColor }}>{report.score}/100</Text>
          </Text>
          <Text>
            🎯 {t("stockAnalysis.dashboard.confidence")}: <Text strong>{report.confidence.toFixed(0)}%</Text>
          </Text>
        </Space>
        <Progress
          percent={report.score}
          strokeColor={scoreColor}
          size="small"
          style={{ marginBottom: 8 }}
        />
        <ReportMarkdown content={report.coreConclusion} isDark={isDark} />
      </Card>

      {/* 2. 买卖点位 */}
      <Card size="small" title={t("stockAnalysis.dashboard.buySellPoints")} style={{ marginTop: 12 }}>
        <Space orientation="vertical" style={{ width: "100%" }}>
          {report.buyPointLow !== null && report.buyPointLow !== undefined
            && report.buyPointHigh !== null
            && report.buyPointHigh !== undefined && (
            <Text>
              {t("stockAnalysis.dashboard.buyRange")}:{" "}
              <Text strong>
                {fmtNum(report.buyPointLow)} - {fmtNum(report.buyPointHigh)}
              </Text>
            </Text>
          )}
          {/* 交易目标价（LLM trader 的方向性目标）—— 与下方「内在价值」是两个不同概念 */}
          <Text>
            {t("stockAnalysis.dashboard.targetPriceTrading")}: {targetPriceMeaningful
              ? <Text strong style={{ color: "#f5222d" }}>{fmtNum(report.targetPrice)}</Text>
              : <Text type="secondary">{t("stockAnalysis.dashboard.targetPriceUnset")}</Text>}
          </Text>
          {
            /* 内在价值区间（估值语义，来自 t-valuation 的 DCF 三档）——
              与交易目标价并列展示，消除「同一工作流两个目标价」的误读 */
          }
          {intrinsicRange && (
            <Text>
              {t("stockAnalysis.dashboard.intrinsicValue")}:{" "}
              <Text strong style={{ color: "#1677ff" }}>{intrinsicRange}</Text>
              {report.currentPrice != null && (
                <Text type="secondary">
                  {" ("}
                  {t("stockAnalysis.dashboard.vsCurrentPrice")} {fmtNum(report.currentPrice)}
                  {")"}
                </Text>
              )}
            </Text>
          )}
          {
            /* 2026-09-22: 区间口径标注 —— 用户实证质问「十几块到四十几块有什么用」。
              该区间的宽度几乎全部来自**增长率假设**（同一个 FCF 锚 ×0.6/×1.0/×1.5），
              它不是「公司值 low~high 元」的概率区间。不标注口径 = 让读者把假设扫描
              当成估值结论（`AUDIT-300642-run-variance-2026-09-22.md`）。 */
          }
          {intrinsicRange && report.intrinsicValueLow != null && report.intrinsicValueHigh != null && (
            <Text type="secondary" className="text-xs">
              {t("stockAnalysis.dashboard.intrinsicValueBandNote")}
            </Text>
          )}
          <Text>
            {t("stockAnalysis.dashboard.stopLoss")}:{" "}
            <Text strong style={{ color: "#52c41a" }}>{fmtNum(report.stopLoss)}</Text>
          </Text>
          <Text>
            {t("stockAnalysis.dashboard.positionPct")}: <Text strong>{report.positionPct.toFixed(0)}%</Text>
          </Text>
        </Space>
      </Card>

      {/* 3. 风险警报 */}
      <RiskAlertsSection alerts={report.riskAlerts} />

      {/* 4. 催化因素 */}
      <CatalystsSection catalysts={report.catalysts} />

      {/* 5. 操作检查清单 */}
      <ChecklistSection items={report.checklist} />

      {/* 6. 最新动态 */}
      {report.latestNews && (
        <Card size="small" title={t("stockAnalysis.dashboard.latestNews")} style={{ marginTop: 12 }}>
          <ReportMarkdown content={report.latestNews} isDark={isDark} />
        </Card>
      )}

      {/* 7. 业绩预期 */}
      {report.earningsExpectation && (
        <Card
          size="small"
          title={t("stockAnalysis.dashboard.earningsExpectation")}
          style={{ marginTop: 12 }}
        >
          <ReportMarkdown content={report.earningsExpectation} isDark={isDark} />
        </Card>
      )}
    </div>
  );
}
