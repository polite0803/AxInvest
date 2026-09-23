import { List } from "@/components/common/AntdList";
import { useStockJump } from "@/hooks/useStockJump";
import { invoke } from "@/lib/invoke";
import { getActionTKey, resolveDisplayAction } from "@/lib/stock-analysis-utils";
import { useStockAnalysisStore } from "@/stores";
import { ArrowRightOutlined, HistoryOutlined } from "@ant-design/icons";
import { Button, Card, Spin, Tag, Tooltip } from "antd";
import { RotateCcw } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { useNavigate } from "react-router-dom";
import { PanelEmpty, type PanelEmptyKind } from "./PanelEmpty";

interface DecisionComparison {
  analysisDate: string;
  /** 决策档位（方向强度轴，中文存储形态：买入 / 增持 / 持有 / 观望 / 减持 / 卖出 …） */
  action: string;
  /**
   * 决策持仓状态轴（与 `action` 正交）。
   *
   * `null` = 该记录产生于 v228 之前，**采集时点没有这个信息** ——
   * 由 `resolveDisplayAction` 退回 `positionPct` 派生，不得读成 `EMPTY`。
   */
  positionState: string | null;
  /** 决策仓位权重（%）；`null` 同为「信息缺失」，不是 0。 */
  positionPct: number | null;
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
  changePct: number;
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

function DecisionBadge({ decision }: { decision: DecisionComparison }) {
  const { t } = useTranslation();
  // 展示档必须走统一入口：`action` 是「方向强度」轴，与持仓状态正交，
  // 它不是可直接展示的档位名 —— 直接渲染 `decision.action` 会让同一条决策
  // 在历史卡显示「观望」、在这里显示「持有」（2026-09-21 实证的同源矛盾）。
  // 2026-09-22: 展示档已收敛为**方向档恒等**（不再按仓位派生，见
  // `AUDIT-300642-run-variance-2026-09-22.md`），逻辑仍只保留一处。
  const displayAction = resolveDisplayAction(
    decision.action,
    decision.positionState,
    decision.positionPct,
  );
  const actionLabel = t(getActionTKey(displayAction));
  let statusText: string;
  let statusColor: string = "default";
  if (decision.stopLossHit) {
    statusText = t("stockAnalysis.dailyReview.stopLossHit");
    statusColor = "red";
  } else if (decision.targetHit) {
    statusText = t("stockAnalysis.dailyReview.targetHit");
    statusColor = "green";
  } else if (decision.inTargetZone) {
    statusText = t("stockAnalysis.dailyReview.inZone");
    statusColor = "gold";
  } else { statusText = `${decision.daysSinceAnalysis}d`; }

  return (
    <Tooltip
      title={
        <div className="text-[11px] space-y-0.5">
          <div>{t("stockAnalysis.dailyReview.lastAnalysis")}: {decision.analysisDate}</div>
          <div>{t("stockAnalysis.dailyReview.decisionAction")}: {actionLabel}</div>
          {decision.targetPrice && <div>{t("stockAnalysis.dailyReview.target")}: {decision.targetPrice.toFixed(2)}
          </div>}
          {decision.stopLoss && <div>{t("stockAnalysis.dailyReview.stopLoss")}: {decision.stopLoss.toFixed(2)}</div>}
        </div>
      }
    >
      <Tag color={statusColor} className="m-0 text-[10px]">{actionLabel} {statusText}</Tag>
    </Tooltip>
  );
}

export function DailyReviewPanel() {
  const { t } = useTranslation();
  const navigate = useNavigate();
  // 跳转统一走 useStockJump（URL 是当前股票的唯一真相源）
  const jumpToStock = useStockJump();

  const [review, setReview] = useState<DailyReview | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const generate = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const r = await invoke<DailyReview>("generate_daily_review");
      setReview(r);
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    }
    setLoading(false);
  }, []);

  useEffect(() => {
    invoke<DailyReview>("generate_daily_review")
      .then((r) => {
        setReview(r);
        setError(null);
      })
      .catch((e: unknown) => {
        setError(e instanceof Error ? e.message : String(e));
      })
      .finally(() => setLoading(false));
  }, [setReview, setError, setLoading]);

  const hasWatchlist = useStockAnalysisStore((s) => s.watchlistVersion > 0);
  const emptyKind: PanelEmptyKind = error ? "connectionFailed" : "noData";
  const emptyDescription = !hasWatchlist
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
                    <List.Item style={{ padding: "3px 0" }}>
                      <div className="text-xs w-full flex items-center justify-between gap-2">
                        <div
                          className="flex items-center gap-1.5 min-w-0 flex-1"
                          style={{ cursor: "pointer" }}
                          onClick={() => jumpToStock({ code: w.stockCode, name: w.stockName })}
                        >
                          <Tag className="m-0 text-[10px]">{w.stockCode}</Tag>
                          <span className="font-medium truncate">{w.stockName}</span>
                          <span style={{ color: w.changePct >= 0 ? "var(--sa-red)" : "var(--sa-green)" }}>
                            {w.changePct >= 0 ? "+" : ""}
                            {w.changePct.toFixed(2)}%
                          </span>
                          {w.lastDecision && <DecisionBadge decision={w.lastDecision} />}
                        </div>
                        <div className="flex gap-1 shrink-0">
                          <Tooltip title={t("stockAnalysis.dailyReview.reflect")}>
                            <Button
                              size="small"
                              type="text"
                              icon={<RotateCcw size={11} />}
                              onClick={() => jumpToStock({ code: w.stockCode, name: w.stockName })}
                            />
                          </Tooltip>
                          <Tooltip title={t("stockAnalysis.dailyReview.backtest")}>
                            <Button
                              size="small"
                              type="text"
                              icon={<HistoryOutlined style={{ fontSize: 11 }} />}
                              onClick={() => navigate(`/backtest?code=${w.stockCode}`)}
                            />
                          </Tooltip>
                          <ArrowRightOutlined style={{ fontSize: 10, color: "var(--muted)", alignSelf: "center" }} />
                        </div>
                      </div>
                      {(w.keyEvents.length > 0 || w.alertTriggers.length > 0) && (
                        <div className="flex flex-wrap gap-1 mt-1">
                          {w.keyEvents.map((e, i) => (
                            <Tag key={`ke-${i}`} color="orange" className="text-[10px] m-0">{e}</Tag>
                          ))}
                          {w.alertTriggers.map((a, i) => (
                            <Tag key={`at-${i}`} color="red" className="text-[10px] m-0">{a}</Tag>
                          ))}
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
