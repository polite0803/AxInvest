import { invoke } from "@/lib/invoke";
import { useStockAnalysisStore } from "@/stores";
import { Button, Card, List, Spin, Tag, Typography } from "antd";
import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { PanelEmpty, type PanelEmptyKind } from "./PanelEmpty";

interface DailyReview {
  date: string;
  summary: string;
  watchlistAnalyses: { stockCode: string; stockName: string; suggestion: string }[];
  alerts: string[];
  recommendations: string[];
}

export function DailyReviewPanel() {
  const { t } = useTranslation();
  const watchlistVersion = useStockAnalysisStore((s) => s.watchlistVersion);
  const [codes, setCodes] = useState<string[]>([]);
  const [review, setReview] = useState<DailyReview | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // 跟随自选股变化
  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const list: any[] = await invoke("list_watchlist");
        if (!cancelled) { setCodes(Array.isArray(list) ? list.map((w) => w.stockCode) : []); }
      } catch {
        if (!cancelled) { setCodes([]); }
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [watchlistVersion]);

  const generate = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const r = await invoke<DailyReview>("generate_daily_review", { codes });
      setReview(r);
    } catch (e: any) {
      setError(e?.message ?? String(e));
    }
    setLoading(false);
  }, [codes]);

  const hasCodes = codes.length > 0;
  const emptyKind: PanelEmptyKind = error ? "connectionFailed" : "noData";
  const emptyDescription = !hasCodes
    ? t("stockAnalysis.dailyReview.noWatchlist")
    : (error ?? t("stockAnalysis.dailyReview.empty"));

  return (
    <Card
      size="small"
      title={t("stockAnalysis.dailyReview.title")}
      styles={{ body: { padding: "8px 10px" } }}
      extra={
        <Button
          size="small"
          loading={loading}
          onClick={generate}
          type={review ? "default" : "primary"}
          disabled={loading}
        >
          {review ? t("stockAnalysis.dailyReview.regenerate") : t("stockAnalysis.dailyReview.generate")}
        </Button>
      }
    >
      {loading
        ? <Spin size="small" style={{ display: "block", margin: "16px auto" }} />
        : !review
        ? <PanelEmpty kind={emptyKind} description={emptyDescription} />
        : (
          <div className="space-y-2">
            {review.summary && (
              <Typography.Paragraph className="text-xs" style={{ marginBottom: 4 }}>
                {review.summary}
              </Typography.Paragraph>
            )}

            {review.alerts?.length > 0 && (
              <div>
                <div className="text-xs font-medium mb-1">
                  {t("stockAnalysis.dailyReview.alerts")} ({review.alerts.length})
                </div>
                <div className="flex flex-wrap gap-1">
                  {review.alerts.map((a, i) => <Tag key={i} color="red" className="text-xs m-0">{a}</Tag>)}
                </div>
              </div>
            )}

            {review.watchlistAnalyses?.length > 0 && (
              <div>
                <div className="text-xs font-medium mb-1">
                  {t("stockAnalysis.dailyReview.watchlist", { count: review.watchlistAnalyses.length })}
                </div>
                <List
                  size="small"
                  dataSource={review.watchlistAnalyses}
                  renderItem={(w) => (
                    <List.Item style={{ padding: "3px 0" }}>
                      <div className="text-xs w-full">
                        <span className="font-mono mr-1">{w.stockCode}</span>
                        <span className="font-medium mr-2">{w.stockName}</span>
                        <span className="text-gray-500">{w.suggestion}</span>
                      </div>
                    </List.Item>
                  )}
                />
              </div>
            )}

            {review.recommendations?.length > 0 && (
              <div>
                <div className="text-xs font-medium mb-1">{t("stockAnalysis.dailyReview.recommendations")}</div>
                <div className="flex flex-wrap gap-1">
                  {review.recommendations.map((r, i) => <Tag key={i} color="blue" className="text-xs m-0">{r}</Tag>)}
                </div>
              </div>
            )}
          </div>
        )}
    </Card>
  );
}
