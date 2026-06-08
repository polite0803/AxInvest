import { ReplayBadge, ReplayWatermark } from "@/components/time-travel/ReplayBadge";
import { invoke } from "@/lib/invoke";
import { useTimeAnchorStore } from "@/stores/feature/timeAnchorStore";
import type { BacktestStats } from "@/types/stock-analysis";
import { Alert, Button, Card, Empty, Segmented, Spin, Statistic } from "antd";
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
    </Card>
  );
}
