import { invoke } from "@/lib/invoke";
import { computeStockConsensus } from "@/lib/stock-analysis-utils";
import { useSettingsStore, useStockAnalysisStore } from "@/stores";
import { useTimeAnchorStore } from "@/stores/feature/timeAnchorStore";
import { getActionColor, getActionTKey, getRiskColor, getRiskTKey } from "@/lib/stock-analysis-utils";
import { ExpandOutlined } from "@ant-design/icons";
import { Button, Card, message, Modal, Tag } from "antd";
import NodeRenderer from "markstream-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { useNavigate } from "react-router-dom";
import { cleanToolCallTags } from "./utils";

export function DecisionBanner() {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const themeMode = useSettingsStore((s) => s.settings.theme_mode);
  const isDark = themeMode === "dark"
    || (themeMode === "system" && window.matchMedia("(prefers-color-scheme: dark)").matches);
  const decision = useStockAnalysisStore((s) => s.decision);
  const stockCode = useStockAnalysisStore((s) => s.stockCode);
  const stockName = useStockAnalysisStore((s) => s.stockName);
  const quote = useStockAnalysisStore((s) => s.quote);
  const analystReports = useStockAnalysisStore((s) => s.analystReports);
  const debateRounds = useStockAnalysisStore((s) => s.debateRounds);
  const riskAssessments = useStockAnalysisStore((s) => s.riskAssessments);
  const bumpWatchlistVersion = useStockAnalysisStore((s) => s.bumpWatchlistVersion);
  const watchlistVersion = useStockAnalysisStore((s) => s.watchlistVersion);
  // 时间旅行: 当前决策所基于的 as-of 锚点 (live 时为 null)
  const asOfDate = useTimeAnchorStore((s) => s.asOfDate);
  const [adding, setAdding] = useState(false);
  const [watchlisted, setWatchlisted] = useState(false);
  const [expanded, setExpanded] = useState(false);
  // 快速交易录入 ref（替代 getElementById，防止多实例 ID 冲突）
  const tradePriceRef = useRef<HTMLInputElement>(null);
  const tradeQtyRef = useRef<HTMLInputElement>(null);

  // stockCode 或自选列表变化时同步自选状态
  useEffect(() => {
    if (!stockCode) { return; }
    let cancelled = false;
    (async () => {
      try {
        const list = await invoke<{ stockCode: string }[]>("list_watchlist");
        if (!cancelled) {
          setWatchlisted(list.some((w) => w.stockCode === stockCode));
        }
      } catch {
        if (!cancelled) { setWatchlisted(false); }
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [stockCode, watchlistVersion]);

  const addToWatchlist = useCallback(async () => {
    if (!stockCode || !stockName) { return; }
    setAdding(true);
    try {
      await invoke("add_to_watchlist", { stockCode, stockName });
      setWatchlisted(true);
      bumpWatchlistVersion();
      message.success(t("stockAnalysis.addedToWatchlist"));
    } catch {
      message.error(t("stockAnalysis.addFailed"));
    }
    setAdding(false);
  }, [stockCode, stockName, t, bumpWatchlistVersion]);

  const actionLabel = (action: string) => t(getActionTKey(action));

  // 共享的决策上下文计算（避免 handleExport / handleAskAI 重复计算）
  const decisionContext = useMemo(() => {
    if (!decision || !stockCode || !stockName) { return null; }
    const currentPrice = quote?.price ?? 0;
    const targetPriceNum = decision.targetPrice != null ? Number(decision.targetPrice) : 0;
    const upside = targetPriceNum > 0 && currentPrice > 0
      ? ((targetPriceNum - currentPrice) / currentPrice * 100)
      : null;
    const confidencePct = Math.round(decision.confidence ?? 0);
    return { currentPrice, targetPriceNum, upside, confidencePct };
  }, [decision, stockCode, stockName, quote]);

  // Hooks 必须在 early return 之前 — 闭包内部自己处理 null decision
  const handleExport = useCallback(() => {
    const ctx = decisionContext;
    if (!ctx || !decision || !stockCode || !stockName) { return; }
    const { currentPrice, upside, confidencePct } = ctx;
    const lines = [
      t("stockAnalysis.export.title"),
      t("stockAnalysis.export.stock", { name: stockName, code: stockCode }),
      t("stockAnalysis.export.date", { date: new Date().toLocaleDateString() }),
      t("stockAnalysis.export.price", { price: currentPrice.toFixed(2) }),
      t("stockAnalysis.export.decision", { action: decision.action, confidence: confidencePct }),
      t("stockAnalysis.export.target", { target: decision.targetPrice ?? "-", stopLoss: decision.stopLoss ?? "-" }),
      t("stockAnalysis.export.position", { pct: decision.positionPct, risk: t(getRiskTKey(decision.riskLevel)) }),
      upside != null ? t("stockAnalysis.export.upside", { pct: (upside >= 0 ? "+" : "") + upside.toFixed(1) }) : "",
      ``,
      t("stockAnalysis.export.reasoning"),
      decision.reasoning,
      ``,
      t("stockAnalysis.export.analystReports", { count: Object.keys(analystReports).length }),
      t("stockAnalysis.export.debateRounds", { count: debateRounds.length }),
      t("stockAnalysis.export.riskAssessments", { count: Object.keys(riskAssessments).length }),
    ].filter(Boolean).join("\n");

    const blob = new Blob([lines], { type: "text/plain;charset=utf-8" });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = `AxInvest_${stockCode}_${new Date().toISOString().slice(0, 10)}.txt`;
    a.click();
    URL.revokeObjectURL(url);
    message.success(t("stockAnalysis.exported"));
  }, [decision, stockCode, stockName, decisionContext, t, analystReports, debateRounds, riskAssessments]);

  const handleAskAI = useCallback(() => {
    const ctx = decisionContext;
    if (!ctx || !decision || !stockCode || !stockName) { return; }
    const { currentPrice, upside, confidencePct } = ctx;
    const context = [
      t("stockAnalysis.askAi.prompt", { stockName, code: stockCode }),
      decision ? t("stockAnalysis.askAi.decision", { action: decision.action, confidence: confidencePct }) : "",
      t("stockAnalysis.export.price", { price: currentPrice.toFixed(2) }),
      upside != null ? t("stockAnalysis.export.upside", { pct: (upside >= 0 ? "+" : "") + upside.toFixed(1) }) : "",
      `${t(getRiskTKey(decision.riskLevel))}`,
    ].filter(Boolean).join("\n");

    navigator.clipboard.writeText(context).then(() => {
      message.success(t("stockAnalysis.contextCopied"));
      navigate(`/chat?code=${stockCode}`);
    }).catch(() => {
      navigate(`/chat?code=${stockCode}`);
    });
  }, [decision, stockCode, stockName, decisionContext, navigate, t]);

  // ── 空决策检查：全 0 / 无目标价 / 无理由 时不渲染 ──
  if (!decision) { return null; }
  // TypeScript 在此之后已将 decision 收窄为 StockDecision (non-null)
  const emptyDecision = decision.confidence === 0
    && decision.positionPct === 0
    && decision.targetPrice == null
    && decision.stopLoss == null
    && (!decision.reasoning || decision.reasoning.trim() === "");
  if (emptyDecision) { return null; }

  // ── 决策 vs 分析师共识矛盾检测 ──
  const isContradictory = (() => {
    if (!analystReports || Object.keys(analystReports).length < 3 || !decision) { return false; }
    const consensus = computeStockConsensus(analystReports, undefined, decision.timeHorizon);
    const isBullishAction = decision.action === "BUY" || decision.action === "INCREASE";
    const isBearishAction = decision.action === "SELL" || decision.action === "REDUCE";
    if (isBullishAction && (consensus.consensus === "bearish" || consensus.consensus === "divided")) { return true; }
    if (isBearishAction && (consensus.consensus === "bullish" || consensus.consensus === "divided")) { return true; }
    return false;
  })();

  const confidencePct = Math.round(decision.confidence ?? 0);
  const meterColor = confidencePct >= 70
    ? "var(--sa-green)"
    : confidencePct >= 40
    ? "var(--sa-amber)"
    : "var(--sa-red)";

  // 从报价和决策计算预期收益
  const currentPrice = quote?.price ?? 0;
  const targetPriceNum = decision.targetPrice != null ? Number(decision.targetPrice) : 0;
  const upside = targetPriceNum > 0 && currentPrice > 0
    ? ((targetPriceNum - currentPrice) / currentPrice * 100)
    : null;

  return (
    <>
      <Card
        size="small"
        title={
          <div className="flex items-center gap-2">
            <span>{t("stockAnalysis.finalDecision")}</span>
            <Tag color={getActionColor(decision.action)}>
              {actionLabel(decision.action)}
            </Tag>
            {asOfDate && (
              <Tag color="purple" title={t("timeTravel.replayBadge.tooltip", { date: asOfDate })}>
                ⏪ {t("timeTravel.pageAnchor.untilDate", { date: asOfDate })}
              </Tag>
            )}
            {isContradictory && (
              <Tag color="orange">
                ⚠️ {t("stockAnalysis.contradiction")}
              </Tag>
            )}
            {decision.timeHorizon && (
              <Tag color="geekblue">
                {t(`stockAnalysis.timeHorizon${
                  decision.timeHorizon === "ultra_short"
                    ? "UltraShort"
                    : decision.timeHorizon === "short"
                    ? "Short"
                    : decision.timeHorizon === "mid"
                    ? "Mid"
                    : "Long"
                }`)}
              </Tag>
            )}
          </div>
        }
        extra={
          <Button
            type="text"
            size="small"
            icon={<ExpandOutlined />}
            onClick={() => setExpanded(true)}
          />
        }
        styles={{ body: { padding: "12px 16px" } }}
        style={{ borderLeft: "4px solid var(--accent)" }}
      >
        <div className="mb-3">
          <div className="flex justify-between text-xs mb-1">
            <span style={{ color: "var(--muted)" }}>{t("stockAnalysis.confidence")}</span>
            <span className="font-mono font-semibold" style={{ color: meterColor, fontSize: 15 }}>
              {confidencePct}%
            </span>
          </div>
          <div
            className="relative"
            style={{ height: 10, borderRadius: 5, background: "var(--surface)", overflow: "hidden" }}
          >
            <div
              style={{
                width: `${confidencePct}%`,
                height: "100%",
                borderRadius: 5,
                background: `linear-gradient(to right, ${meterColor}88, ${meterColor})`,
                transition: "width 0.6s ease",
              }}
            />
          </div>
        </div>

        <div
          className="sa-markdown-content text-xs mb-3 p-2 rounded"
          style={{ background: "var(--surface)" }}
        >
          {cleanToolCallTags(decision.reasoning || "")
            ? <NodeRenderer content={cleanToolCallTags(decision.reasoning || "")} isDark={isDark} />
            : <span style={{ color: "var(--muted)" }}>{t("stockAnalysis.noDecisionReasoning")}</span>}
        </div>

        <div
          className="grid gap-2 mb-3"
          style={{ gridTemplateColumns: "repeat(3, 1fr)" }}
        >
          {decision.targetPrice && (
            <div className="text-center p-1.5 rounded" style={{ background: "var(--surface)" }}>
              <div className="text-xs" style={{ color: "var(--muted)" }}>
                {t("stockAnalysis.targetPrice")}
              </div>
              <div className="text-sm font-semibold font-mono">
                ¥{decision.targetPrice}
              </div>
            </div>
          )}
          {decision.stopLoss && (
            <div className="text-center p-1.5 rounded" style={{ background: "var(--surface)" }}>
              <div className="text-xs" style={{ color: "var(--muted)" }}>
                {t("stockAnalysis.stopLoss")}
              </div>
              <div className="text-sm font-semibold font-mono" style={{ color: "var(--sa-red)" }}>
                ¥{decision.stopLoss}
              </div>
            </div>
          )}
          <div className="text-center p-1.5 rounded" style={{ background: "var(--surface)" }}>
            <div className="text-xs" style={{ color: "var(--muted)" }}>{t("stockAnalysis.position")}</div>
            <div className="text-sm font-semibold font-mono">{decision.positionPct}%</div>
          </div>
          {upside != null && (
            <div className="text-center p-1.5 rounded" style={{ background: "var(--surface)" }}>
              <div className="text-xs" style={{ color: "var(--muted)" }}>
                {t("stockAnalysis.expectedUpside")}
              </div>
              <div
                className="text-sm font-semibold font-mono"
                style={{ color: upside >= 0 ? "var(--sa-green)" : "var(--sa-red)" }}
              >
                {upside >= 0 ? "+" : ""}
                {upside.toFixed(1)}%
              </div>
            </div>
          )}
          <div className="text-center p-1.5 rounded" style={{ background: "var(--surface)" }}>
            <div className="text-xs" style={{ color: "var(--muted)" }}>{t("stockAnalysis.riskLevel")}</div>
            <div
              className="text-sm font-semibold"
              style={{
                color: getRiskColor(decision.riskLevel),
              }}
            >
              {t(getRiskTKey(decision.riskLevel))}
            </div>
          </div>
          {decision.expectedHoldingDays && (
            <div className="text-center p-1.5 rounded" style={{ background: "var(--surface)" }}>
              <div className="text-xs" style={{ color: "var(--muted)" }}>{t("stockAnalysis.expectedHoldingDays")}</div>
              <div className="text-sm font-semibold font-mono">{decision.expectedHoldingDays}天</div>
            </div>
          )}
          {decision.targetTimeframe && (
            <div className="text-center p-1.5 rounded" style={{ background: "var(--surface)" }}>
              <div className="text-xs" style={{ color: "var(--muted)" }}>{t("stockAnalysis.targetTimeframe")}</div>
              <div className="text-sm font-semibold font-mono">{decision.targetTimeframe}</div>
            </div>
          )}
        </div>

        {/* 快速交易录入 — 决策日可直接在此录入买卖 */}
        {stockCode && decision.action && decision.action !== "HOLD"
          && (
            <div
              className="flex items-center gap-2 p-2 mt-2 rounded"
              style={{ background: "var(--surface)" }}
            >
              <span className="text-xs whitespace-nowrap" style={{ color: "var(--muted)" }}>
                {t("stockAnalysis.trade.quickRecord")}
              </span>
              <input
                type="number"
                placeholder={t("trade.price")}
                defaultValue={decision.targetPrice ?? undefined}
                ref={tradePriceRef}
                className="text-xs"
                style={{
                  width: 70,
                  padding: "2px 6px",
                  border: "1px solid var(--color-border-tertiary)",
                  borderRadius: 4,
                  background: "transparent",
                  color: "var(--color-text-primary)",
                }}
              />
              <input
                type="number"
                placeholder={t("trade.quantity")}
                defaultValue={100}
                ref={tradeQtyRef}
                className="text-xs"
                style={{
                  width: 60,
                  padding: "2px 6px",
                  border: "1px solid var(--color-border-tertiary)",
                  borderRadius: 4,
                  background: "transparent",
                  color: "var(--color-text-primary)",
                }}
              />
              <Button
                size="small"
                type="primary"
                style={{ fontSize: 11, lineHeight: "18px", height: 22, padding: "0 8px" }}
                onClick={async () => {
                  const price = parseFloat(tradePriceRef.current?.value ?? "0");
                  const qty = parseInt(tradeQtyRef.current?.value ?? "100", 10);
                  if (price <= 0 || qty <= 0) { return; }
                  const analysisId = useStockAnalysisStore.getState().analysisId;
                  try {
                    await invoke("record_trade", {
                      stockCode,
                      stockName,
                      direction: decision.action === "SELL" ? "sell" : "buy",
                      price,
                      quantity: Math.round(qty / 100) * 100,
                      tradeDate: new Date().toISOString().slice(0, 10),
                      tradeTime: new Date().toISOString().slice(11, 16),
                      notes: `${t("stockAnalysis.trade.fromDecision")} (${
                        t("stockAnalysis.confidence")
                      }: ${confidencePct}%)`,
                      analysisId: analysisId ?? null,
                    });
                    message.success(t("trade.recorded"));
                  } catch (e: unknown) {
                    message.error(String(e));
                  }
                }}
              >
                {t("stockAnalysis.trade.record")}
              </Button>
            </div>
          )}

        <div className="flex gap-2 items-center flex-wrap">
          {stockCode && !watchlisted && (
            <Button size="small" type="dashed" loading={adding} onClick={addToWatchlist}>
              ⭐ {t("stockAnalysis.addToWatchlist")}
            </Button>
          )}
          {watchlisted && <Tag color="gold">⭐ {t("stockAnalysis.inWatchlist")}</Tag>}
          {stockCode && (
            <>
              <Button size="small" icon={<span>💬</span>} onClick={handleAskAI}>
                {t("stockAnalysis.askAI")}
              </Button>
              <Button size="small" icon={<span>📥</span>} onClick={handleExport}>
                {t("stockAnalysis.exportReport")}
              </Button>
            </>
          )}
        </div>
      </Card>

      <Modal
        title={
          <div className="flex items-center gap-2">
            <span>{t("stockAnalysis.finalDecision")}</span>
            <Tag color={getActionColor(decision.action)}>
              {actionLabel(decision.action)}
            </Tag>
            {asOfDate && (
              <Tag color="purple" title={t("timeTravel.replayBadge.tooltip", { date: asOfDate })}>
                ⏪ {t("timeTravel.pageAnchor.untilDate", { date: asOfDate })}
              </Tag>
            )}
          </div>
        }
        open={expanded}
        onCancel={() => setExpanded(false)}
        footer={null}
        width="80vw"
        style={{ top: 20 }}
        styles={{ body: { maxHeight: "80vh", overflow: "auto" } }}
      >
        <div className="mb-4">
          <div className="flex justify-between mb-1">
            <span style={{ color: "var(--muted)" }}>{t("stockAnalysis.confidence")}</span>
            <span className="font-mono font-semibold" style={{ color: meterColor, fontSize: 20 }}>
              {confidencePct}%
            </span>
          </div>
          <div
            className="relative"
            style={{ height: 14, borderRadius: 7, background: "var(--surface)", overflow: "hidden" }}
          >
            <div
              style={{
                width: `${confidencePct}%`,
                height: "100%",
                borderRadius: 7,
                background: `linear-gradient(to right, ${meterColor}88, ${meterColor})`,
                transition: "width 0.6s ease",
              }}
            />
          </div>
        </div>

        <div
          className="grid gap-3 mb-4"
          style={{ gridTemplateColumns: "repeat(5, 1fr)" }}
        >
          {decision.targetPrice && (
            <div className="text-center p-3 rounded" style={{ background: "var(--surface)" }}>
              <div className="text-xs" style={{ color: "var(--muted)" }}>{t("stockAnalysis.targetPrice")}</div>
              <div className="text-lg font-semibold font-mono">¥{decision.targetPrice}</div>
            </div>
          )}
          {decision.stopLoss && (
            <div className="text-center p-3 rounded" style={{ background: "var(--surface)" }}>
              <div className="text-xs" style={{ color: "var(--muted)" }}>{t("stockAnalysis.stopLoss")}</div>
              <div className="text-lg font-semibold font-mono" style={{ color: "var(--sa-red)" }}>
                ¥{decision.stopLoss}
              </div>
            </div>
          )}
          <div className="text-center p-3 rounded" style={{ background: "var(--surface)" }}>
            <div className="text-xs" style={{ color: "var(--muted)" }}>{t("stockAnalysis.position")}</div>
            <div className="text-lg font-semibold font-mono">{decision.positionPct}%</div>
          </div>
          {upside != null && (
            <div className="text-center p-3 rounded" style={{ background: "var(--surface)" }}>
              <div className="text-xs" style={{ color: "var(--muted)" }}>{t("stockAnalysis.expectedUpside")}</div>
              <div
                className="text-lg font-semibold font-mono"
                style={{ color: upside >= 0 ? "var(--sa-green)" : "var(--sa-red)" }}
              >
                {upside >= 0 ? "+" : ""}
                {upside.toFixed(1)}%
              </div>
            </div>
          )}
          <div className="text-center p-3 rounded" style={{ background: "var(--surface)" }}>
            <div className="text-xs" style={{ color: "var(--muted)" }}>{t("stockAnalysis.riskLevel")}</div>
            <div
              className="text-lg font-semibold"
              style={{
                color: getRiskColor(decision.riskLevel),
              }}
            >
              {t(getRiskTKey(decision.riskLevel))}
            </div>
          </div>
        </div>

        <div
          className="sa-markdown-content text-sm mb-4 p-4 rounded"
          style={{ background: "var(--surface)" }}
        >
          {cleanToolCallTags(decision.reasoning || "")
            ? <NodeRenderer content={cleanToolCallTags(decision.reasoning || "")} isDark={isDark} />
            : <span style={{ color: "var(--muted)" }}>{t("stockAnalysis.noDecisionReasoning")}</span>}
        </div>

        <div className="flex gap-2 items-center flex-wrap">
          {stockCode && !watchlisted && (
            <Button type="dashed" loading={adding} onClick={addToWatchlist}>
              ⭐ {t("stockAnalysis.addToWatchlist")}
            </Button>
          )}
          {watchlisted && <Tag color="gold">⭐ {t("stockAnalysis.inWatchlist")}</Tag>}
          {stockCode && (
            <>
              <Button icon={<span>💬</span>} onClick={handleAskAI}>
                {t("stockAnalysis.askAI")}
              </Button>
              <Button icon={<span>📥</span>} onClick={handleExport}>
                {t("stockAnalysis.exportReport")}
              </Button>
            </>
          )}
        </div>
      </Modal>
    </>
  );
}
