import { invoke } from "@/lib/invoke";
import type { BacktestStats } from "@/types/stock-analysis";
import { Button, Card, Empty, Spin, Statistic } from "antd";
import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

export function BacktestPanel() {
  const { t } = useTranslation();
  const [stats, setStats] = useState<BacktestStats | null>(null);
  const [loading, setLoading] = useState(false);
  const [fetchError, setFetchError] = useState(false);
  const holdingDays = 5;

  const load = useCallback(async () => {
    setLoading(true);
    setFetchError(false);
    try {
      const result = await invoke<BacktestStats>("backtest_all_history", { holdingDays });
      setStats(result);
    } catch {
      setFetchError(true);
    }
    setLoading(false);
  }, [holdingDays]);

  useEffect(() => {
    load();
  }, [load]);

  const accuracyColor = stats ? stats.accuracyPct >= 60 ? "var(--sa-red)" : "var(--sa-green)" : undefined;
  const returnColor = stats && stats.avgReturnPct >= 0 ? "var(--sa-red)" : "var(--sa-green)";

  return (
    <Card
      size="small"
      title={`📊 ${t("stockAnalysis.backtest.title")}`}
      styles={{ body: { padding: "8px 10px" } }}
      extra={
        <Button size="small" loading={loading} onClick={load}>
          {t("stockAnalysis.retry")}
        </Button>
      }
    >
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
    </Card>
  );
}
