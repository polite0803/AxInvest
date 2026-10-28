// i18n-exempt: 业务逻辑判断字符串，非 UI 展示文本
import {
  AlertOutlined,
  ClusterOutlined,
  ReloadOutlined,
  SafetyOutlined,
  ThunderboltOutlined,
  WarningOutlined,
} from "@ant-design/icons";
import { Alert, Button, Col, Empty, Progress, Row, Skeleton, Space, Statistic, Tag, Tooltip } from "antd";
import { useEffect, useMemo, useRef } from "react";
import { useTranslation } from "react-i18next";

import { portfolioRiskColor, portfolioRiskTKey } from "@/lib/stock-analysis-utils";
import { type PortfolioCorrelationCell, useStockAnalysisStore } from "@/stores/feature/stockAnalysisStore";
import { useTimeAnchorStore } from "@/stores/feature/timeAnchorStore";

function fmtMoney(n: number | undefined): string {
  if (n === undefined || n === null) { return "—"; }
  return `${n.toFixed(2)}`;
}

function fmtPct(n: number | undefined, withSign = true): string {
  if (n === undefined || n === null) { return "—"; }
  const sign = n > 0 && withSign ? "+" : "";
  return `${sign}${n.toFixed(2)}%`;
}

function correlationColor(c: number): string {
  if (c >= 0.7) { return "#cf1322"; }
  if (c >= 0.4) { return "#d46b08"; }
  if (c <= -0.3) { return "#389e0d"; }
  return "#8c8c8c";
}

// 2026-09-21: 本文件原有的本地 `riskColor`（`level.includes("高") ? "red" : …`）已删除，
//   改调权威函数，避免「同一入参两处结论不同」。
// 2026-10-15: 但当时的权威 `getRiskColor`/`getRiskTKey` 是**决策链**那套档位（低/中/高/极高，
//   由 rhai `overall_risk` 产出），而本面板的 `dashboard.riskLevel` 来自**集中度算法**
//   （`compute_risk_level`：低/中等/中高/高 + 无持仓）。两套体系串味后，「中高风险」在
//   决策链判据里归「中」、在本文件的子串表里归「高」——同一个词两个结论。
//   故改用本面板自己那套的 `portfolioRiskColor`/`portfolioRiskTKey`，判据与文案同源。

export function PortfolioMonitorPanel() {
  const { t } = useTranslation();
  const asOfDate = useTimeAnchorStore((s) => s.asOfDate);
  const mode = useTimeAnchorStore((s) => s.mode);
  const dashboard = useStockAnalysisStore((s) => s.portfolioDashboard);
  const correlations = useStockAnalysisStore((s) => s.portfolioCorrelations);
  const correlationsError = useStockAnalysisStore((s) => s.portfolioCorrelationsError);
  const refreshing = useStockAnalysisStore((s) => s.portfolioRefreshing);
  const lastError = useStockAnalysisStore((s) => s.portfolioLastError);
  const fetchDashboard = useStockAnalysisStore((s) => s.fetchPortfolioDashboard);
  const fetchCorrelations = useStockAnalysisStore((s) => s.fetchPortfolioCorrelations);
  const refresh = useStockAnalysisStore((s) => s.refreshPortfolioMetrics);
  const lastAsOfRef = useRef<string | null>(null);

  useEffect(() => {
    // 首次加载或 asOfDate 变化时重新拉取数据
    if (lastAsOfRef.current !== asOfDate) {
      lastAsOfRef.current = asOfDate;
      void fetchDashboard(asOfDate);
      void fetchCorrelations(asOfDate);
    }
  }, [asOfDate, fetchDashboard, fetchCorrelations]);

  const isReplay = mode === "replay" && !!asOfDate;

  const hasDashboard = dashboard !== null && typeof dashboard === "object" && !Array.isArray(dashboard);

  const sortedSectors = useMemo(() => {
    if (!hasDashboard || !dashboard || !dashboard.sectorExposure) { return []; }
    return Object.entries(dashboard.sectorExposure)
      .sort((a, b) => b[1] - a[1])
      .slice(0, 8);
  }, [dashboard, hasDashboard]);

  const sortedCorrelations = useMemo(() => {
    if (!correlations || !Array.isArray(correlations)) { return []; }
    const cells = [...correlations];
    cells.sort((a, b) => Math.abs(b.correlation) - Math.abs(a.correlation));
    return cells.slice(0, 10);
  }, [correlations]);

  if (!hasDashboard && !refreshing) {
    return (
      <div className="p-4">
        <Empty description={t("stockAnalysis.portfolioMonitor.empty")} />
        {lastError && (
          <Alert
            type="error"
            showIcon
            className="mt-2"
            title={t("stockAnalysis.portfolioMonitor.loadError")}
            description={lastError}
          />
        )}
        <Button
          type="primary"
          icon={<ReloadOutlined />}
          onClick={() => refresh(asOfDate)}
          className="mt-3"
        >
          {t("stockAnalysis.portfolioMonitor.refresh")}
        </Button>
      </div>
    );
  }

  if (!hasDashboard) {
    return <Skeleton active paragraph={{ rows: 6 }} />;
  }

  if (!dashboard) {
    return <Skeleton active paragraph={{ rows: 6 }} />;
  }

  return (
    <div className="space-y-3 p-3">
      {isReplay && (
        <Alert
          type="info"
          showIcon
          title={t("stockAnalysis.portfolioMonitor.historicalTitle", { date: dashboard.asOfDate ?? asOfDate })}
          description={t("stockAnalysis.portfolioMonitor.historicalDesc")}
        />
      )}

      {dashboard.concentrationWarning && (
        <Alert
          type="warning"
          showIcon
          icon={<WarningOutlined />}
          title={t("stockAnalysis.portfolioMonitor.warningTitle")}
          description={dashboard.concentrationWarning}
        />
      )}

      {
        /* G-1：集中度与分散度现在只按「拿到现价」的那几只算 ⇒ 缺价的只数必须跟着说出来，
          否则「按 5 只算出的分散度」会配着 8 行持仓显示，同一张面板两套口径。
          句子刻意不带「按几只算」那个数：历史快照路径不返回持仓明细，凑出来的 0 只会是假数。 */
      }
      {dashboard.unpricedPositions > 0 && (
        <Alert
          type="warning"
          showIcon
          title={t("stockAnalysis.portfolioMonitor.unpricedTitle")}
          description={t("stockAnalysis.portfolioMonitor.unpricedNotice", {
            count: dashboard.unpricedPositions,
          })}
        />
      )}

      {/* 1) Metric Cards */}
      <Row gutter={[12, 12]}>
        <Col xs={12} sm={8} md={4}>
          <Statistic
            title={t("stockAnalysis.portfolioMonitor.totalMv")}
            value={fmtMoney(dashboard.totalMarketValue)}
            prefix="¥"
            styles={{ content: { fontSize: 18 } }}
          />
        </Col>
        <Col xs={12} sm={8} md={4}>
          <Statistic
            title={t("stockAnalysis.portfolioMonitor.totalPnl")}
            value={fmtMoney(dashboard.totalPnl)}
            styles={{
              content: {
                fontSize: 18,
                color: dashboard.totalPnl >= 0 ? "var(--red)" : "var(--green)",
              },
            }}
            prefix={dashboard.totalPnl >= 0 ? "+" : ""}
          />
        </Col>
        <Col xs={12} sm={8} md={4}>
          <Statistic
            title={t("stockAnalysis.portfolioMonitor.totalPnlPct")}
            value={fmtPct(dashboard.totalPnlPct, false)}
            styles={{
              content: {
                fontSize: 18,
                color: dashboard.totalPnlPct >= 0 ? "var(--red)" : "var(--green)",
              },
            }}
          />
        </Col>
        <Col xs={12} sm={8} md={4}>
          <Statistic
            title={t("stockAnalysis.portfolioMonitor.topConcentration")}
            value={fmtPct(dashboard.topConcentrationPct, false)}
            styles={{ content: { fontSize: 18 } }}
            suffix={
              <Tooltip title={t("stockAnalysis.portfolioMonitor.topConcentrationTip")}>
                <Tag color="red" className="ml-1">
                  {dashboard.positions.length}
                </Tag>
              </Tooltip>
            }
          />
        </Col>
        <Col xs={12} sm={8} md={4}>
          <Tooltip title={t("stockAnalysis.portfolioMonitor.betaTip")}>
            <Statistic
              title={t("stockAnalysis.portfolioMonitor.beta")}
              value={dashboard.beta?.toFixed(2) ?? "—"}
              styles={{ content: { fontSize: 18 } }}
            />
          </Tooltip>
        </Col>
        <Col xs={12} sm={8} md={4}>
          <Tooltip title={t("stockAnalysis.portfolioMonitor.diversificationTip")}>
            <Statistic
              title={t("stockAnalysis.portfolioMonitor.diversificationScore")}
              value={dashboard.diversificationScore}
              suffix="/ 100"
              styles={{
                content: {
                  fontSize: 18,
                  color: dashboard.diversificationScore >= 70 ? "var(--green)" : "var(--amber)",
                },
              }}
            />
          </Tooltip>
        </Col>
      </Row>

      <Row gutter={[12, 12]} className="items-center">
        <Col flex="auto">
          <Space>
            <Tag color={portfolioRiskColor(dashboard.riskLevel)} icon={<SafetyOutlined />}>
              {t("stockAnalysis.portfolioMonitor.riskLevel")}: {t(portfolioRiskTKey(dashboard.riskLevel))}
            </Tag>
            {dashboard.correlationAvg != null && (
              <Tag color="purple" icon={<ClusterOutlined />}>
                {t("stockAnalysis.portfolioMonitor.correlationAvg")}: {dashboard.correlationAvg.toFixed(2)}
              </Tag>
            )}
            {dashboard.sharpe30d != null && (
              <Tag color="blue">
                {t("stockAnalysis.portfolioMonitor.sharpe30d")}: {dashboard.sharpe30d.toFixed(2)}
              </Tag>
            )}
            <span className="text-xs text-gray-500">
              {t("stockAnalysis.portfolioMonitor.snapshotAt")}: {dashboard.snapshotAt
                ? new Date(dashboard.snapshotAt).toLocaleString()
                : "—"}
            </span>
          </Space>
        </Col>
        <Col>
          <Button
            icon={<ReloadOutlined />}
            onClick={() => refresh(asOfDate)}
            loading={refreshing}
            type="primary"
          >
            {t("stockAnalysis.portfolioMonitor.refresh")}
          </Button>
        </Col>
      </Row>

      {/* 2) 行业集中度 + 相关性高亮 */}
      <Row gutter={[12, 12]}>
        <Col xs={24} md={12}>
          <div className="rounded border border-gray-200 bg-white p-3 dark:border-gray-700 dark:bg-gray-900">
            <div className="mb-2 flex items-center gap-2">
              <ClusterOutlined />
              <span className="font-semibold">{t("stockAnalysis.portfolioMonitor.sectorExposure")}</span>
            </div>
            {sortedSectors.length === 0 ? <Empty description={t("stockAnalysis.portfolioMonitor.noSector")} /> : (
              sortedSectors.map(([sector, pct]) => (
                <div key={sector} className="mb-2">
                  <div className="flex justify-between text-xs">
                    <span>{sector}</span>
                    <span className="text-gray-500">{pct.toFixed(1)}%</span>
                  </div>
                  <Progress
                    percent={Math.min(100, pct)}
                    showInfo={false}
                    strokeColor={pct > 40 ? "#cf1322" : pct > 25 ? "#d46b08" : "#1677ff"}
                    size="small"
                  />
                </div>
              ))
            )}
          </div>
        </Col>

        <Col xs={24} md={12}>
          <div className="rounded border border-gray-200 bg-white p-3 dark:border-gray-700 dark:bg-gray-900">
            <div className="mb-2 flex items-center gap-2">
              <AlertOutlined />
              <span className="font-semibold">{t("stockAnalysis.portfolioMonitor.correlationHighlights")}</span>
            </div>
            {/* 三态各占一句：读取失败 ≠ 没有相关性数据（后者会让人以为「确实没有」） */}
            {correlationsError
              ? (
                <Alert
                  type="warning"
                  showIcon
                  message={t("stockAnalysis.settings.vendor.connectionFailed")}
                />
              )
              : sortedCorrelations.length === 0
              ? <Empty description={t("stockAnalysis.portfolioMonitor.noCorrelation")} />
              : (
                <ul className="space-y-1 text-sm">
                  {sortedCorrelations.map((c: PortfolioCorrelationCell) => (
                    <li
                      key={`${c.codeA}-${c.codeB}`}
                      className="flex items-center justify-between"
                    >
                      <span>
                        <span className="font-mono">{c.codeA}</span>
                        <span className="mx-1 text-gray-400">↔</span>
                        <span className="font-mono">{c.codeB}</span>
                      </span>
                      <span
                        className="font-mono font-semibold"
                        style={{ color: correlationColor(c.correlation) }}
                      >
                        {c.correlation.toFixed(2)}
                      </span>
                    </li>
                  ))}
                </ul>
              )}
          </div>
        </Col>
      </Row>

      {/* 3) 压测三场景 */}
      <div className="rounded border border-gray-200 bg-white p-3 dark:border-gray-700 dark:bg-gray-900">
        <div className="mb-2 flex items-center gap-2">
          <ThunderboltOutlined />
          <span className="font-semibold">{t("stockAnalysis.portfolioMonitor.stressTest")}</span>
        </div>
        <Row gutter={[12, 12]}>
          {[dashboard.stressTest.m10, dashboard.stressTest.m20, dashboard.stressTest.blackSwan]
            .filter(Boolean)
            .map((s) => {
              if (!s) { return null; }
              return (
                <Col key={s.scenario} xs={24} sm={8}>
                  <div className="rounded border border-gray-200 p-2 dark:border-gray-700">
                    <div className="text-sm font-semibold">{s.label}</div>
                    <div
                      className="text-2xl font-bold"
                      style={{ color: s.portfolioPnl < 0 ? "var(--red)" : "var(--green)" }}
                    >
                      {fmtMoney(s.portfolioPnl)}
                      {t("stockAnalysis.portfolio.yuan")}
                    </div>
                    <div className="text-xs text-gray-500">
                      {fmtPct(s.portfolioPnlPct, false)} · {t("stockAnalysis.portfolioMonitor.worstHit")}:{" "}
                      {s.topHit?.stockCode ?? "—"}
                      {s.topHit ? ` (${fmtPct(s.topHit.pnlPct, false)})` : ""}
                    </div>
                    <div className="mt-1 text-xs text-gray-400">{s.note}</div>
                  </div>
                </Col>
              );
            })}
        </Row>
      </div>
    </div>
  );
}
