import { invoke } from "@/lib/invoke";
import { computeStockConsensus } from "@/lib/stock-analysis-utils";
import { getActionColor, getActionTKey, getRiskColor, getRiskTKey } from "@/lib/stock-analysis-utils";
import { useSettingsStore, useStockAnalysisStore } from "@/stores";
import { useTimeAnchorStore } from "@/stores/feature/timeAnchorStore";
import { ExpandOutlined } from "@ant-design/icons";
import { App, Button, Card, Modal, Tag } from "antd";
import NodeRenderer from "markstream-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { useNavigate } from "react-router-dom";
import { cleanToolCallTags } from "./utils";

export function DecisionBanner() {
  const { message } = App.useApp();
  const { t } = useTranslation();
  const navigate = useNavigate();
  const themeMode = useSettingsStore((s) => s.settings.theme_mode);
  const isDark = themeMode === "dark"
    || (themeMode === "system" && window.matchMedia("(prefers-color-scheme: dark)").matches);
  const decision = useStockAnalysisStore((s) => s.decision);
  const stockCode = useStockAnalysisStore((s) => s.stockCode);
  const stockName = useStockAnalysisStore((s) => s.stockName);
  // 重跑分析: 透传当前 analysisId 让后端"覆盖"同 id 旧记录(而非新建一条)
  const analysisId = useStockAnalysisStore((s) => s.analysisId);
  // 加载失败错误(loadAnalysis reject 时由 StockAnalysisPage 写入)
  const error = useStockAnalysisStore((s) => s.error);
  const quote = useStockAnalysisStore((s) => s.quote);
  const analystReports = useStockAnalysisStore((s) => s.analystReports);
  const debateRounds = useStockAnalysisStore((s) => s.debateRounds);
  const riskAssessments = useStockAnalysisStore((s) => s.riskAssessments);
  const bumpWatchlistVersion = useStockAnalysisStore((s) => s.bumpWatchlistVersion);
  const watchlistVersion = useStockAnalysisStore((s) => s.watchlistVersion);
  // 方案 D 双向并存: LLM 决策 JSON + 一致性分数
  const llmDecisionJson = useStockAnalysisStore((s) => s.llmDecisionJson);
  const decisionAgreementScore = useStockAnalysisStore((s) => s.decisionAgreementScore);
  // 分歧阈值（从工作流模板读取,默认 40）
  const [disagreementThreshold, setDisagreementThreshold] = useState(40);
  useEffect(() => {
    invoke<Record<string, unknown>>("get_workflow_template", { id: "stock-analysis" })
      .then((tmpl) => {
        const vars = (tmpl?.variables ?? []) as Record<string, unknown>[];
        const v = vars.find((x: Record<string, unknown>) => x.name === "dual_view_disagreement_threshold");
        if (v && typeof v.value === "number") {
          setDisagreementThreshold(v.value);
        }
      })
      .catch(() => {});
  }, []);
  // 解析 LLM stance + summary 用于 banner 展示
  const llmStance = useMemo(() => {
    if (!llmDecisionJson) { return null; }
    try {
      const j = JSON.parse(llmDecisionJson);
      return j.stance ?? null;
    } catch {
      return null;
    }
  }, [llmDecisionJson]);
  const llmSummary = useMemo(() => {
    if (!llmDecisionJson) { return null; }
    try {
      const j = JSON.parse(llmDecisionJson);
      return j.summary ?? null;
    } catch {
      return null;
    }
  }, [llmDecisionJson]);
  // timeline-jump 高亮：被 evidence 指向时短暂加 ring 样式
  const highlightedPanel = useStockAnalysisStore((s) => s.highlightedPanel);
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
  }, [stockCode, stockName, t, bumpWatchlistVersion, message]);

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
  }, [decision, stockCode, stockName, decisionContext, t, analystReports, debateRounds, riskAssessments, message]);

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
  }, [decision, stockCode, stockName, decisionContext, navigate, t, message]);

  // ── 决策缺失占位 ──
  // normalizeDecision 入口已保证非空对象，所以这里的 !decision 意味着：
  //   1) LLM 输出无法解析出 decision（portfolio-mgr 节点结果残缺/被截断）
  //   2) 决策 JSON 是全零空壳（loadAnalysis 已主动跳过 set）
  //   3) loadAnalysis 本身失败（store.error 有值，stockCode 同步预填过但记录详情加载失败）
  // 此时不再 return null，而是渲染"决策缺失"占位卡，让用户知道工作流
  // 已完成但决策信息不完整，并提供"重跑 / 重试"入口。
  if (!decision) {
    // 修复：loadAnalysis 失败时优先渲染"加载失败 + 重试"，避免永远停在
    // "搜索股票"分支(用户认为这违反"history 已有 stockCode"的预期)。
    // StockAnalysisPage 的 useEffect 已从 history 缓存同步预填 stockCode，
    // 所以 error 状态下 stockCode 通常是有值的 → 走"重试"分支。
    if (error) {
      return (
        <Card
          id="decision-banner-top"
          size="small"
          styles={{ body: { padding: "12px 16px" } }}
          style={{
            borderLeft: "4px solid var(--sa-red, #ef4444)",
          }}
          data-testid="decision-banner-load-failed"
        >
          <div className="flex items-start gap-2">
            <span style={{ fontSize: 18, lineHeight: 1 }}>⚠️</span>
            <div className="flex-1">
              <div className="text-sm font-semibold mb-1">
                {t("stockAnalysis.loadAnalysisFailed")}
              </div>
              <div
                className="text-xs mb-2"
                style={{ color: "var(--muted)", wordBreak: "break-all" }}
              >
                {error}
              </div>
              <Button
                size="small"
                type="primary"
                onClick={() => {
                  // 重试：清掉 error 状态后重新调用 loadAnalysis
                  useStockAnalysisStore.setState({ error: null });
                  if (analysisId) {
                    void useStockAnalysisStore.getState().loadAnalysis(analysisId);
                  }
                }}
              >
                {t("stockAnalysis.retry")}
              </Button>
            </div>
          </div>
        </Card>
      );
    }
    return (
      <Card
        id="decision-banner-top"
        size="small"
        styles={{ body: { padding: "12px 16px" } }}
        style={{
          borderLeft: "4px solid var(--sa-amber, #f59e0b)",
          ...(highlightedPanel === "decision"
            ? { boxShadow: "0 0 0 3px var(--accent)", transition: "box-shadow 0.4s" }
            : {}),
        }}
        data-testid="decision-banner-missing"
      >
        <div className="flex items-start gap-2">
          <span style={{ fontSize: 18, lineHeight: 1 }}>⚠️</span>
          <div className="flex-1">
            <div className="text-sm font-semibold mb-1">
              {t("stockAnalysis.decisionMissing")}
            </div>
            <div className="text-xs" style={{ color: "var(--muted)" }}>
              {t("stockAnalysis.decisionMissingHint")}
            </div>
            {stockCode
              ? (
                <div className="mt-2">
                  <Button
                    size="small"
                    type="primary"
                    onClick={() =>
                      useStockAnalysisStore.getState().startAnalysis(
                        stockCode,
                        // 重跑覆盖原 analysisId 对应记录(不传则后端新建 UUID)。
                        // 从决策缺失占位卡点出时 store.analysisId 必然是已点开的历史记录 id。
                        analysisId ? { replaceAnalysisId: analysisId } : undefined,
                      )}
                  >
                    {t("stockAnalysis.reAnalyze")}
                  </Button>
                </div>
              )
              : (
                <div className="mt-2 flex items-center gap-2">
                  <Button
                    size="small"
                    type="primary"
                    onClick={() => {
                      // 占位阶段 stockCode 还没就绪(极少见:用户从未打开过 history 缓存,
                      // 也未在搜索栏选过股票,直接通过分享链接进入):
                      // 聚焦顶部搜索栏让用户输入。
                      const search = document.querySelector<HTMLInputElement>(
                        "[data-testid='stock-analysis-search-input']",
                      );
                      search?.focus();
                      message.info(t("stockAnalysis.reAnalyzeNeedCode"));
                    }}
                  >
                    {t("stockAnalysis.searchStock")}
                  </Button>
                  <span className="text-xs" style={{ color: "var(--muted)" }}>
                    {t("stockAnalysis.reAnalyzeNeedCodeHint")}
                  </span>
                </div>
              )}
          </div>
        </div>
      </Card>
    );
  }

  // TypeScript 在此之后已将 decision 收窄为 StockDecision (non-null)

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
        id="decision-banner-top"
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
        style={{
          borderLeft: "4px solid var(--accent)",
          ...(highlightedPanel === "decision"
            ? { boxShadow: "0 0 0 3px var(--accent)", transition: "box-shadow 0.4s" }
            : {}),
        }}
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

        {/* [Phase 2 step 10] 分歧时 LLM 对比标注 */}
        {decisionAgreementScore !== null && decisionAgreementScore < disagreementThreshold && llmSummary && (
          <div
            className="text-xs mb-3 p-2 rounded"
            style={{ background: "rgba(124, 58, 237, 0.08)", borderLeft: "3px solid #7c3aed" }}
          >
            <span className="font-medium" style={{ color: "#7c3aed" }}>
              💡 LLM 视角:
            </span>
            <span className="ml-1" style={{ color: "var(--color-text-secondary)" }}>
              {llmSummary}
            </span>
          </div>
        )}

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

      {/* [Phase 2] 决策一致性胶囊: 点击展开双视角对比 */}
      {decisionAgreementScore !== null && (
        <div
          className="flex items-center gap-2 px-3 py-1.5 rounded cursor-pointer hover:opacity-80 transition-opacity mt-1"
          style={{
            background: decisionAgreementScore >= 60
              ? "rgba(16, 185, 129, 0.1)"
              : decisionAgreementScore >= 40
              ? "rgba(245, 158, 11, 0.1)"
              : "rgba(239, 68, 68, 0.1)",
          }}
          onClick={() => {
            // 用事件总线通知 StockAnalysisPage 切换 tab
            window.dispatchEvent(new CustomEvent("switch-tab", { detail: "decision-comparison" }));
          }}
        >
          <span className="text-[11px]" style={{ color: "var(--muted)" }}>
            📊 {t("stockAnalysis.dualViewConsistency")}
          </span>
          <span
            className="font-mono text-[12px] font-semibold"
            style={{
              color: decisionAgreementScore >= 60
                ? "#10b981"
                : decisionAgreementScore >= 40
                ? "#f59e0b"
                : "#ef4444",
            }}
          >
            {decisionAgreementScore}/100
          </span>
          {llmStance && (
            <>
              <span className="text-[11px]" style={{ color: "var(--muted)" }}>·</span>
              <span
                className="px-1.5 rounded text-[10px] font-medium"
                style={{ background: "var(--sa-purple-bg, #ede9fe)", color: "#7c3aed" }}
              >
                LLM: {llmStance}
              </span>
            </>
          )}
          {decisionAgreementScore < disagreementThreshold && (
            <span className="text-[10px]" style={{ color: "#ef4444" }}>
              ⚠️ {t("stockAnalysis.dualViewDisagreement")}
            </span>
          )}
        </div>
      )}

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

        {/* [Phase 2 step 10] Modal 内分歧时 LLM 对比标注 */}
        {decisionAgreementScore !== null && decisionAgreementScore < disagreementThreshold && llmSummary && (
          <div
            className="text-sm mb-4 p-3 rounded"
            style={{ background: "rgba(124, 58, 237, 0.08)", borderLeft: "3px solid #7c3aed" }}
          >
            <span className="font-medium" style={{ color: "#7c3aed" }}>
              💡 LLM 视角:
            </span>
            <span className="ml-1" style={{ color: "var(--color-text-secondary)" }}>
              {llmSummary}
            </span>
          </div>
        )}

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
