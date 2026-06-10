import { invoke } from "@/lib/invoke";
import { parseAction, StockAction } from "@/types";
import { useStockAnalysisStore } from "@/stores";
import { useNavigate } from "react-router-dom";
import { ArrowRightOutlined } from "@ant-design/icons";
import { Button, Card, List, Spin, Tag, Tooltip } from "antd";
import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { PanelEmpty, type PanelEmptyKind } from "./PanelEmpty";

interface DecisionComparison {
  analysisDate: string;
  action: string;
  targetPrice: number | null;
  stopLoss: number | null;
  daysSinceAnalysis: number;
  inTargetZone: boolean;
  stopLossHit: boolean;
  targetHit: boolean;
}

interface StockDaySummary {
  stockCode: string;
  stockName: string;
  open: number;
  high: number;
  low: number;
  close: number;
  changePct: number;
  volumeRatio: number | null;
  keyEvents: string[];
  alertTriggers: string[];
  lastDecision: DecisionComparison | null;
}

interface DailyReview {
  date: string;
  marketStatus: string;
  watchlistSummary: StockDaySummary[];
  generatedAt: string;
}

/** 决策对比徽章 — 显示上次分析结论 + 当前状态 */
function DecisionBadge({ decision }: { decision: DecisionComparison }) {
  const { t } = useTranslation();
  const action = parseAction(decision.action);
  let statusText = "";
  let statusColor = "default";
  if (decision.stopLossHit) { statusText = "⚠ " + t("stockAnalysis.dailyReview.stopLossHit"); statusColor = "red"; }
  else if (decision.targetHit) { statusText = "✓ " + t("stockAnalysis.dailyReview.targetHit"); statusColor = "green"; }
  else if (decision.inTargetZone) { statusText = "🎯 " + t("stockAnalysis.dailyReview.inZone"); statusColor = "gold"; }
  else { statusText = `${decision.daysSinceAnalysis}d`; statusColor = "default"; }

  return (
    <Tooltip
      title={
        <div className="text-[11px] space-y-0.5">
          <div>{t("stockAnalysis.dailyReview.lastAnalysis")}: {decision.analysisDate}</div>
          <div>{t("stockAnalysis.dailyReview.decisionAction")}: {decision.action}</div>
          {decision.targetPrice && <div>🎯 {t("stockAnalysis.dailyReview.target")}: {decision.targetPrice.toFixed(2)}</div>}
          {decision.stopLoss && <div>🛡 {t("stockAnalysis.dailyReview.stopLoss")}: {decision.stopLoss.toFixed(2)}</div>}
          <div>{t("stockAnalysis.dailyReview.daysSince")}: {decision.daysSinceAnalysis}d</div>
        </div>
      }
    >
      <Tag color={statusColor} className="m-0 text-[10px]" style={{ cursor: "pointer" }}>
        {action === StockAction.BUY || action === StockAction.INCREASE
          ? t("stockAnalysis.actionBuy")
          : action === StockAction.SELL || action === StockAction.REDUCE
          ? t("stockAnalysis.actionSell")
          : t("stockAnalysis.actionHold")}{" "}
        {statusText}
      </Tag>
    </Tooltip>
  );
}

export function DailyReviewPanel() {
  const { t } = useTranslation();
  const navigate = useNavigate();
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
      const r = await invoke<DailyReview>("generate_daily_review");
      setReview(r);
    } catch (e: any) {
      setError(e?.message ?? String(e));
    }
    setLoading(false);
  }, []);

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
            {/* 市场状态 */}
            <div className="text-xs flex items-center gap-2">
              <span className="text-gray-500">{review.date}</span>
              <Tag color="default" className="m-0 text-[10px]">{review.marketStatus}</Tag>
            </div>

            {review.watchlistSummary.length > 0 && (
              <div>
                <div className="text-xs font-medium mb-1">
                  {t("stockAnalysis.dailyReview.watchlist", { count: review.watchlistSummary.length })}
                </div>
                <List
                  size="small"
                  dataSource={review.watchlistSummary}
                  renderItem={(w) => (
                    <List.Item
                      style={{ padding: "3px 0", cursor: "pointer" }}
                      onClick={() => navigate(`/stock-analysis?code=${w.stockCode}`)}
                    >
                      <div className="text-xs w-full flex items-center justify-between gap-2">
                        <div className="flex items-center gap-1.5 min-w-0 flex-1">
                          <Tag className="m-0 text-[10px]">{w.stockCode}</Tag>
                          <span className="font-medium truncate">{w.stockName}</span>
                          <span style={{ color: w.changePct >= 0 ? "var(--sa-red)" : "var(--sa-green)" }}>
                            {w.changePct >= 0 ? "+" : ""}{w.changePct.toFixed(2)}%
                          </span>
                          {w.lastDecision && <DecisionBadge decision={w.lastDecision} />}
                        </div>
                        <ArrowRightOutlined style={{ fontSize: 10, color: "var(--muted)" }} />
                      </div>
                      {(w.keyEvents.length > 0 || w.alertTriggers.length > 0) && (
                        <div className="flex flex-wrap gap-1 mt-1">
                          {w.keyEvents.map((e, i) => <Tag key={`ke-${i}`} color="orange" className="text-[10px] m-0">{e}</Tag>)}
                          {w.alertTriggers.map((a, i) => <Tag key={`at-${i}`} color="red" className="text-[10px] m-0">{a}</Tag>)}
                        </div>
                      )}
                    </List.Item>
                  )}
                />
              </div>
            )}
          </div>
        )}
    </Card>
  );
}
