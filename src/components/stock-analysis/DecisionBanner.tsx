import { invoke } from "@/lib/invoke";
import { useSettingsStore, useStockAnalysisStore } from "@/stores";
import { Button, Card, message, Tag } from "antd";
import NodeRenderer from "markstream-react";
import { useCallback, useEffect, useMemo, useState } from "react";
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
  const [adding, setAdding] = useState(false);
  const [watchlisted, setWatchlisted] = useState(false);

  // stockCode 变化时同步自选状态
  useEffect(() => {
    if (typeof window !== "undefined" && stockCode) {
      setWatchlisted(window.localStorage.getItem("ax_watchlisted") === stockCode);
    }
  }, [stockCode]);

  const addToWatchlist = useCallback(async () => {
    if (!stockCode || !stockName) { return; }
    setAdding(true);
    try {
      await invoke("add_to_watchlist", { stockCode, stockName });
      setWatchlisted(true);
      if (typeof window !== "undefined") { window.localStorage.setItem("ax_watchlisted", stockCode); }
      bumpWatchlistVersion();
      message.success(t("stockAnalysis.addedToWatchlist"));
    } catch {
      message.error(t("stockAnalysis.addFailed"));
    }
    setAdding(false);
  }, [stockCode, stockName, t, bumpWatchlistVersion]);

  const actionLabel: Record<string, string> = {
    "买入": t("stockAnalysis.actionBuy"),
    "增持": t("stockAnalysis.actionIncrease"),
    "持有": t("stockAnalysis.actionHold"),
    "减持": t("stockAnalysis.actionReduce"),
    "卖出": t("stockAnalysis.actionSell"),
  };

  const confidencePct = useMemo(() => Math.round(decision?.confidence ?? 0), [decision]);
  const meterColor = confidencePct >= 70
    ? "var(--sa-green)"
    : confidencePct >= 40
    ? "var(--sa-amber)"
    : "var(--sa-red)";

  // 从报价和决策计算预期收益
  const currentPrice = quote?.price ?? 0;
  const targetPriceNum = decision?.targetPrice != null ? Number(decision.targetPrice) : 0;
  const upside = targetPriceNum > 0 && currentPrice > 0
    ? ((targetPriceNum - currentPrice) / currentPrice * 100)
    : null;

  // 导出报告
  const handleExport = useCallback(() => {
    if (!decision || !stockCode || !stockName) { return; }
    const lines = [
      `=== AxInvest 投资分析报告 ===`,
      `股票: ${stockName} (${stockCode})`,
      `分析日期: ${new Date().toLocaleDateString("zh-CN")}`,
      `当前价: ¥${currentPrice.toFixed(2)}`,
      `决策: ${decision.action} | 置信度: ${confidencePct}%`,
      `目标价: ¥${decision.targetPrice ?? "-"} | 止损: ¥${decision.stopLoss ?? "-"}`,
      `仓位: ${decision.positionPct}% | 风险等级: ${decision.riskLevel}`,
      upside != null ? `预期涨幅: ${upside >= 0 ? "+" : ""}${upside.toFixed(1)}%` : "",
      ``,
      `推理摘要:`,
      decision.reasoning,
      ``,
      `分析师报告: ${Object.keys(analystReports).length} 篇`,
      `辩论轮次: ${debateRounds.length} 轮`,
      `风险评估: ${Object.keys(riskAssessments).length} 项`,
    ].filter(Boolean).join("\n");

    const blob = new Blob([lines], { type: "text/plain;charset=utf-8" });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = `AxInvest_${stockCode}_${new Date().toISOString().slice(0, 10)}.txt`;
    a.click();
    URL.revokeObjectURL(url);
    message.success(t("stockAnalysis.exported"));
  }, [
    decision,
    stockCode,
    stockName,
    currentPrice,
    confidencePct,
    upside,
    analystReports,
    debateRounds,
    riskAssessments,
    t,
  ]);

  // 问 AI：复制股票上下文到剪贴板，跳转到对话页
  const handleAskAI = useCallback(() => {
    if (!stockCode || !stockName) { return; }
    const context = [
      `请分析 ${stockName} (${stockCode}) 的投资前景。`,
      decision ? `最新决策: ${decision.action}, 置信度 ${confidencePct}%。` : "",
      `当前价格: ¥${currentPrice.toFixed(2)}`,
      upside != null ? `预期涨幅: ${upside >= 0 ? "+" : ""}${upside.toFixed(1)}%` : "",
      `风险等级: ${decision?.riskLevel ?? "未知"}`,
    ].filter(Boolean).join("\n");

    navigator.clipboard.writeText(context).then(() => {
      message.success(t("stockAnalysis.contextCopied"));
      navigate(`/chat?code=${stockCode}`);
    }).catch(() => {
      navigate(`/chat?code=${stockCode}`);
    });
  }, [stockCode, stockName, decision, currentPrice, confidencePct, upside, navigate, t]);

  if (!decision) { return null; }

  return (
    <Card
      size="small"
      title={
        <div className="flex items-center gap-2">
          <span>{t("stockAnalysis.finalDecision")}</span>
          <Tag
            color={decision.action === "买入" || decision.action === "增持"
              ? "red"
              : decision.action === "持有"
              ? "blue"
              : decision.action === "减持"
              ? "orange"
              : "green"}
          >
            {actionLabel[decision.action] || decision.action}
          </Tag>
        </div>
      }
      styles={{ body: { padding: "12px 16px" } }}
      style={{ borderLeft: "4px solid var(--accent)" }}
    >
      {/* 信心仪表 */}
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

      {/* 推理摘要 */}
      <div
        className="sa-markdown-content text-xs mb-3 p-2 rounded"
        style={{ background: "var(--surface)" }}
      >
        <NodeRenderer content={cleanToolCallTags(decision.reasoning || "")} isDark={isDark} />
      </div>

      {/* 核心指标网格 — 固定3列、窄屏2列，防止侧栏坍塌 */}
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
              color: String(decision.riskLevel ?? "").includes("高")
                ? "var(--sa-red)"
                : String(decision.riskLevel ?? "").includes("低")
                ? "var(--sa-green)"
                : "var(--sa-amber)",
            }}
          >
            {decision.riskLevel}
          </div>
        </div>
      </div>

      {/* 操作按钮 */}
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
  );
}
