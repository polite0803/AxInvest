import { invoke } from "@/lib/invoke";
import { ReloadOutlined } from "@ant-design/icons";
import { Button, Card, Collapse, Empty, Spin, Tag } from "antd";
import { useCallback, useState } from "react";

interface DailyReview {
  date: string;
  marketSummary: string;
  watchlistReview: Array<{ stockCode: string; stockName: string; comment: string; alertNotes: string[] }>;
  triggeredAlerts: Record<string, string[]>;
  recommendations: string[];
}

export function DailyReviewPanel() {
  const [review, setReview] = useState<DailyReview | null>(null);
  const [loading, setLoading] = useState(false);

  const generate = useCallback(async () => {
    setLoading(true);
    try {
      const r = await invoke<DailyReview>("generate_daily_review");
      setReview(r);
    } catch { /* 后端未运行 */ }
    setLoading(false);
  }, []);

  return (
    <Card
      size="small"
      title="📋 每日复盘"
      styles={{ body: { padding: "6px 8px" } }}
      extra={<Button size="small" icon={<ReloadOutlined />} loading={loading} onClick={generate}>生成</Button>}
    >
      {loading
        ? <Spin size="small" />
        : !review
        ? <Empty image={Empty.PRESENTED_IMAGE_SIMPLE} description="点击生成今日复盘" />
        : (
          <div className="flex flex-col gap-1 text-xs">
            <div className="text-gray-500">{review.date} {review.marketSummary}</div>

            {review.triggeredAlerts && Object.keys(review.triggeredAlerts).length > 0 && (
              <div className="p-1 rounded" style={{ background: "var(--surface)" }}>
                <span className="font-medium">触发告警:</span>
                {Object.entries(review.triggeredAlerts).map(([code, alerts]) => (
                  <div key={code}>{code}: {alerts.join(", ")}</div>
                ))}
              </div>
            )}

            <Collapse
              size="small"
              ghost
              items={[{
                key: "watchlist",
                label: `自选股复盘 (${review.watchlistReview?.length ?? 0})`,
                children: review.watchlistReview?.map((w) => (
                  <div
                    key={w.stockCode}
                    className="flex items-start gap-2 py-0.5"
                    style={{ borderBottom: "1px solid var(--border)" }}
                  >
                    <Tag className="text-xs m-0 shrink-0">{w.stockCode}</Tag>
                    <div>
                      <span className="font-medium">{w.stockName}</span>
                      <div className="text-gray-500">{w.comment}</div>
                      {w.alertNotes.length > 0 && <div className="text-orange-500">{w.alertNotes.join(" ")}</div>}
                    </div>
                  </div>
                )) ?? <Empty image={Empty.PRESENTED_IMAGE_SIMPLE} description="无自选股" />,
              }]}
            />

            {review.recommendations?.length > 0 && (
              <div className="p-1 rounded" style={{ background: "var(--surface)" }}>
                <span className="font-medium">建议:</span>
                {review.recommendations.map((rec, i) => <div key={i}>• {rec}</div>)}
              </div>
            )}
          </div>
        )}
    </Card>
  );
}
