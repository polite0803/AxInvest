import { ReplayBadge, ReplayWatermark } from "@/components/time-travel/ReplayBadge";
import { invoke } from "@/lib/invoke";
import { useTimeAnchorStore } from "@/stores/feature/timeAnchorStore";
import type { BacktestComparisonResponse, BacktestStats, GroupBacktestResult } from "@/types/stock-analysis";
import { Alert, Button, Card, Empty, Segmented, Spin, Statistic, Table, Tag, Tooltip } from "antd";
import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

type BacktestScope = "all" | "live" | "replay";

export function BacktestPanel() {
  const { t } = useTranslation();
  const asOfDate = useTimeAnchorStore((s) => s.asOfDate);
  const anchorMode = useTimeAnchorStore((s) => s.mode);
  const [scope, setScope] = useState<BacktestScope>("all");
  const [stats, setStats] = useState<BacktestStats | null>(null);
  const [loading, setLoading] = useState(false);
  const [fetchError, setFetchError] = useState(false);
  const holdingDays = 5;
  const isReplay = anchorMode === "replay" && asOfDate !== null;

  // ── 策略回测状态 ──
  const [strategyResult, setStrategyResult] = useState<BacktestComparisonResponse | null>(null);
  const [strategyLoading, setStrategyLoading] = useState(false);
  const [strategyError, setStrategyError] = useState<string | null>(null);

  const load = useCallback(async () => {
    setLoading(true);
    setFetchError(false);
    try {
      const result = await invoke<BacktestStats>("backtest_all_history", {
        holdingDays,
        scope,
      });
      setStats(result);
    } catch {
      setFetchError(true);
    }
    setLoading(false);
  }, [holdingDays, scope]);

  useEffect(() => {
    load();
  }, [load]);

  const runStrategyBacktest = useCallback(async () => {
    setStrategyLoading(true);
    setStrategyResult(null);
    setStrategyError(null);
    try {
      const result = await invoke<BacktestComparisonResponse>("backtest_reco_strategies");
      setStrategyResult(result);
    } catch (e: any) {
      setStrategyError(typeof e === "string" ? e : e?.message ?? "回测失败");
    }
    setStrategyLoading(false);
  }, []);

  const accuracyColor = stats ? stats.accuracyPct >= 60 ? "var(--sa-red)" : "var(--sa-green)" : undefined;
  const returnColor = stats && stats.avgReturnPct >= 0 ? "var(--sa-red)" : "var(--sa-green)";

  return (
    <Card
      size="small"
      title={
        <div className="flex items-center gap-2">
          <span>📊 {t("stockAnalysis.backtest.sectionTitle")}</span>
          {isReplay && <ReplayBadge />}
        </div>
      }
      styles={{ body: { padding: "8px 10px" } }}
      extra={
        <Button size="small" loading={loading} onClick={load}>
          {t("stockAnalysis.retry")}
        </Button>
      }
    >
      {isReplay && asOfDate && (
        <Alert
          type="info"
          showIcon
          className="!text-xs !mb-2"
          message={
            <span className="text-xs">
              {t("timeTravel.backtestHint", { date: asOfDate })}
            </span>
          }
        />
      )}
      <div className="mb-2">
        <Segmented
          size="small"
          value={scope}
          onChange={(v) => setScope(v as BacktestScope)}
          options={[
            { label: t("stockAnalysis.backtest.scopeAll"), value: "all" },
            { label: t("stockAnalysis.backtest.scopeLive"), value: "live" },
            { label: t("stockAnalysis.backtest.scopeReplay"), value: "replay" },
          ]}
        />
      </div>
      <div style={{ position: "relative" }}>
        {loading
          ? <Spin size="small" style={{ display: "block", margin: "16px auto" }} />
          : fetchError
          ? <Empty description={t("stockAnalysis.error")} image={Empty.PRESENTED_IMAGE_SIMPLE} />
          : !stats || stats.totalAnalyses === 0
          ? <Empty description={t("stockAnalysis.noRecords")} image={Empty.PRESENTED_IMAGE_SIMPLE} />
          : (
            <div>
              <div className="grid grid-cols-2 gap-2 mb-3">
                <Statistic
                  title={t("stockAnalysis.backtest.total")}
                  value={stats.totalAnalyses}
                  valueStyle={{ fontSize: 18 }}
                />
                <Statistic
                  title={t("stockAnalysis.backtest.accuracy")}
                  value={stats.accuracyPct.toFixed(1)}
                  suffix="%"
                  valueStyle={{ fontSize: 18, color: accuracyColor, fontWeight: "bold" }}
                />
                <Statistic
                  title={t("stockAnalysis.backtest.avgReturn")}
                  value={stats.avgReturnPct.toFixed(2)}
                  suffix="%"
                  valueStyle={{ fontSize: 18, color: returnColor, fontWeight: "bold" }}
                />
                <Statistic
                  title={t("stockAnalysis.backtest.maxDrawdown")}
                  value={stats.avgMaxDrawdownPct.toFixed(2)}
                  suffix="%"
                  valueStyle={{ fontSize: 18, color: "var(--sa-green)" }}
                />
              </div>
              <div className="grid grid-cols-2 gap-2 text-xs text-gray-500">
                <span>{t("stockAnalysis.backtest.avgConfidence")}: {(stats.avgConfidence * 100).toFixed(0)}%</span>
                {stats.alphaPct != null && (
                  <span>{t("stockAnalysis.backtest.alpha")}: {stats.alphaPct.toFixed(2)}%</span>
                )}
              </div>
            </div>
          )}
        {isReplay && <ReplayWatermark />}
      </div>

      {/* ── 荐股策略回测（两组对比） ── */}
      <div className="mt-4 pt-3 border-t border-gray-700/30">
        <div className="flex items-center justify-between mb-2">
          <span className="text-sm font-medium">📈 策略回测（两组对比）</span>
          <Button
            size="small"
            type="primary"
            ghost
            loading={strategyLoading}
            onClick={runStrategyBacktest}
          >
            {strategyLoading ? "回测中..." : "运行回测"}
          </Button>
        </div>

        {strategyError && (
          <Alert
            type="warning"
            showIcon
            className="!text-xs !mb-2"
            message={strategyError}
          />
        )}

        {strategyResult && (
          <>
            <div className="text-xs text-gray-500 mb-2">
              正向: {strategyResult.positive.stockCount} 只 | 负向: {strategyResult.negative.stockCount} 只
              {strategyResult.skipped.length > 0 && (
                <Tooltip title={strategyResult.skipped.join("\n")}>
                  <Tag className="ml-1 cursor-help" color="warning" style={{ fontSize: 10 }}>
                    跳过 {strategyResult.skipped.length} 个
                  </Tag>
                </Tooltip>
              )}
            </div>

            {/* 正向组 */}
            <div className="mb-3">
              <div className="text-xs font-medium text-green-500 mb-1">
                ✅ {strategyResult.positive.label}
              </div>
              <StrategyTable data={strategyResult.positive} />
            </div>

            {/* 负向组 */}
            <div>
              <div className="text-xs font-medium text-orange-500 mb-1">
                ❌ {strategyResult.negative.label}
              </div>
              <StrategyTable data={strategyResult.negative} />
            </div>
          </>
        )}
      </div>
    </Card>
  );
}

/** 单组策略回测表格 */
function StrategyTable({ data }: { data: GroupBacktestResult }) {
  const entries = Object.entries(data.strategies);

  const columns = [
    {
      title: "策略",
      dataIndex: "strategyId",
      key: "strategyId",
      width: 100,
      render: (v: string) => {
        const nameMap: Record<string, string> = {
          trend_short: "趋势·短",
          trend_mid: "趋势·中",
          trend_long: "趋势·长",
          rev_short: "超跌·短",
          rev_mid: "超跌·中",
        };
        return <span className="text-xs font-medium">{nameMap[v] ?? v}</span>;
      },
    },
    {
      title: "信号数",
      dataIndex: "totalSignals",
      key: "totalSignals",
      width: 60,
      render: (v: number) => <span className="text-xs">{v}</span>,
    },
    {
      title: "胜率",
      dataIndex: "winRatePct",
      key: "winRatePct",
      width: 70,
      render: (v: number) => {
        const color = v >= 50 ? "var(--sa-red)" : "var(--sa-green)";
        return <span className="text-xs font-bold" style={{ color }}>{v.toFixed(1)}%</span>;
      },
    },
    {
      title: "平均收益",
      dataIndex: "avgReturnPct",
      key: "avgReturnPct",
      width: 80,
      render: (v: number) => {
        const color = v >= 0 ? "var(--sa-red)" : "var(--sa-green)";
        return <span className="text-xs" style={{ color }}>{v.toFixed(2)}%</span>;
      },
    },
    {
      title: "累计收益",
      dataIndex: "totalReturnPct",
      key: "totalReturnPct",
      width: 80,
      render: (v: number) => {
        const color = v >= 0 ? "var(--sa-red)" : "var(--sa-green)";
        return <span className="text-xs" style={{ color }}>{v.toFixed(1)}%</span>;
      },
    },
    {
      title: "最大回撤",
      dataIndex: "avgMaxDrawdownPct",
      key: "avgMaxDrawdownPct",
      width: 80,
      render: (v: number) => <span className="text-xs" style={{ color: "var(--sa-green)" }}>{v.toFixed(1)}%</span>,
    },
    {
      title: "连亏",
      dataIndex: "maxConsecutiveLosses",
      key: "maxConsecutiveLosses",
      width: 50,
      render: (v: number) => <span className={`text-xs ${v >= 5 ? "text-red-500" : ""}`}>{v}</span>,
    },
    {
      title: "Sharpe",
      dataIndex: "sharpeRatio",
      key: "sharpeRatio",
      width: 65,
      render: (v: number | null) => {
        if (v == null) { return <span className="text-xs text-gray-500">—</span>; }
        const color = v >= 1 ? "#52c41a" : v >= 0 ? "#faad14" : "#ff4d4f";
        return <span className="text-xs font-medium" style={{ color }}>{v.toFixed(2)}</span>;
      },
    },
    {
      title: "盈亏比",
      dataIndex: "profitFactor",
      key: "profitFactor",
      width: 65,
      render: (v: number | null) => {
        if (v == null) { return <span className="text-xs text-gray-500">—</span>; }
        const color = v >= 2 ? "#52c41a" : v >= 1 ? "#faad14" : "#ff4d4f";
        return <span className="text-xs font-medium" style={{ color }}>{v.toFixed(2)}</span>;
      },
    },
  ];

  const dataSource = entries.map(([key, val]) => ({ key, ...val }));

  return (
    <Table
      dataSource={dataSource}
      columns={columns}
      pagination={false}
      size="small"
      bordered={false}
      className="strategy-backtest-table"
    />
  );
}
