import { List } from "@/components/common/AntdList";
import { ReplayBadge, ReplayWatermark } from "@/components/time-travel/ReplayBadge";
import { invoke } from "@/lib/invoke";
import { useStockAnalysisStore } from "@/stores";
import { useTimeAnchorStore } from "@/stores/feature/timeAnchorStore";
import type { LatestAnalysisSummary, StockConsensus } from "@/types";
import type { BacktestComparisonResponse } from "@/types/stock-analysis";
import { parseAction } from "@/types";
import { Alert, Button, Card, Collapse, Empty, Spin, Tabs, Tag, Tooltip } from "antd";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { PanelEmpty, type PanelEmptyKind } from "./PanelEmpty";
import { useStockAnalysisPage } from "./StockAnalysisPageContext";

interface RecommendationPanelProps {
  /**
   * 打开数据源设置的回调。优先于上下文中的实现 —— 让该面板可脱离
   * <StockAnalysisPage> 渲染（例如在选股中心里）。
   * 不传时回退到上下文（默认 no-op）。
   */
  onOpenDataSourceSettings?: () => void;
}

const noop = () => {};

type StyleKey = "trend" | "value" | "capital" | "reversion" | "watchlist";
type PeriodKey = "short" | "mid" | "long";

interface RecoPick {
  stockCode: string;
  stockName: string;
  sector?: string | null;
  style: StyleKey;
  period: PeriodKey;
  price: number;
  entryLow: number;
  entryHigh: number;
  stopLoss: number;
  targetPrice: number;
  positionPct: number;
  holdingDays: number;
  confidence: number;
  reasons: string[];
  riskNotes: string[];
  secondaryStyles?: StyleKey[];
  /** true = 系统初筛 / 数据稀疏兜底（无技术信号），false = 主策略真实命中 */
  synthetic?: boolean;
}

interface RecoResponse {
  period: PeriodKey;
  picks: Partial<Record<StyleKey, RecoPick[]>>;
  disabledStyles: StyleKey[];
  /** as-of 模式下被降级(≠ 缺失)的风格(spec §8)。live 模式恒为空数组。 */
  degradedStyles?: StyleKey[];
  /** degradedStyles 中各风格的具体降级原因,key=styleKey, value=本地化文本 */
  degradedReasons?: Record<string, string>;
  generatedAt: number;
  rawSeedPoolSize: number;
  /** 模式标签: live / replay / backtest_sweep — 后端 spec §8 注入 */
  mode?: string;
  /** 时间旅行模式截止日 YYYY-MM-DD;live 时 undefined */
  asOfDate?: string;
}

const STYLE_KEYS: StyleKey[] = ["trend", "value", "capital", "reversion", "watchlist"];
const STYLE_COLOR: Record<StyleKey, string> = {
  trend: "blue",
  value: "gold",
  capital: "magenta",
  reversion: "green",
  watchlist: "default",
};

const FALLBACK = "—";
const isFiniteNumber = (n: unknown): n is number => typeof n === "number" && Number.isFinite(n);

/** Format a number with decimals; render FALLBACK for non-finite values. */
function fmt(value: unknown, decimals = 2, fallback = FALLBACK): string {
  if (!isFiniteNumber(value)) { return fallback; }
  return value.toFixed(decimals);
}

export function RecommendationPanel({ onOpenDataSourceSettings }: RecommendationPanelProps = {}) {
  const { t, i18n } = useTranslation();
  const { openDataSourceSettings: ctxOpenSettings } = useStockAnalysisPage();
  const openDataSourceSettings = onOpenDataSourceSettings ?? ctxOpenSettings ?? noop;
  const getStockQuote = useStockAnalysisStore((s) => s.getStockQuote);
  const getStockKline = useStockAnalysisStore((s) => s.getStockKline);
  const startAnalysis = useStockAnalysisStore((s) => s.startAnalysis);
  const asOfDate = useTimeAnchorStore((s) => s.asOfDate);
  const anchorMode = useTimeAnchorStore((s) => s.mode);

  const [period, setPeriod] = useState<PeriodKey>("short");
  const [data, setData] = useState<RecoResponse | null>(null);
  const [loading, setLoading] = useState(false);
  const [emptyKind, setEmptyKind] = useState<PanelEmptyKind | null>(null);
  const [generatedAtText, setGeneratedAtText] = useState<string>("");
  // P0-1: 荐股面板关联历史分析数据
  const [latestAnalyses, setLatestAnalyses] = useState<Record<string, LatestAnalysisSummary | null>>({});
  // P0-2: 策略回测统计（每个风格的 win rate + Sharpe）
  const [strategyStats, setStrategyStats] = useState<Record<string, { winRate: number; sharpe: number | null; signalCount: number }> | null>(null);
  const [strategyStatsLoading, setStrategyStatsLoading] = useState(false);

  const reqTokenRef = useRef(0);

  const load = async () => {
    const myToken = ++reqTokenRef.current;
    setLoading(true);
    setEmptyKind(null);
    try {
      const r = await invoke<RecoResponse>("recommend_stocks", { period, asOfDate });
      if (myToken !== reqTokenRef.current) { return; }
      if (!r || !r.picks || Object.keys(r.picks).length === 0) {
        setData(r ?? null);
        if (r && r.disabledStyles && r.disabledStyles.length >= 4) {
          setEmptyKind("vendorDisabled");
        } else {
          setEmptyKind("noData");
        }
        setLoading(false);
        return;
      }
      setData(r);
      const d = new Date(r.generatedAt);
      setGeneratedAtText(
        d.toLocaleTimeString(i18n.language === "zh-CN" ? "zh-CN" : "en-US", {
          hour: "2-digit",
          minute: "2-digit",
        }),
      );

      fetchLatestAnalyses(r);
    } catch (e: any) {
      console.error("[RecommendationPanel] load failed:", e);
      if (myToken !== reqTokenRef.current) { return; }
      setData(null);
      setEmptyKind("connectionFailed");
    }
    if (myToken === reqTokenRef.current) { setLoading(false); }
  };

  useEffect(() => {
    let cancelled = false;
    Promise.resolve().then(() => {
      if (cancelled) return;
      setLoading(true);
      setEmptyKind(null);
      return invoke<RecoResponse>("recommend_stocks", { period, asOfDate });
    }).then((r) => {
        if (cancelled) return;
        if (!r || !r.picks || Object.keys(r.picks).length === 0) {
          setData(r ?? null);
          if (r && r.disabledStyles && r.disabledStyles.length >= 4) {
            setEmptyKind("vendorDisabled");
          } else {
            setEmptyKind("noData");
          }
          return;
        }
        setData(r);
        const d = new Date(r.generatedAt);
        setGeneratedAtText(
          d.toLocaleTimeString(i18n.language === "zh-CN" ? "zh-CN" : "en-US", {
            hour: "2-digit",
            minute: "2-digit",
          }),
        );
        return r;
      })
      .then((r) => {
        if (cancelled || !r) return;
        const allCodes = new Set<string>();
        for (const arr of Object.values(r.picks ?? {})) {
          if (!arr) { continue; }
          for (const p of arr) {
            if (!p.synthetic) { allCodes.add(p.stockCode); }
          }
        }
        if (allCodes.size === 0) { return; }
        return invoke<Record<string, LatestAnalysisSummary | null>>(
          "get_latest_analyses_for_stocks",
          { stockCodes: Array.from(allCodes), asOfDate },
        );
      })
      .then((result) => {
        if (cancelled) return;
        if (result) { setLatestAnalyses(result); }
      })
      .catch((e: any) => {
        console.error("[RecommendationPanel] load failed:", e);
        if (!cancelled) {
          setData(null);
          setEmptyKind("connectionFailed");
        }
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => { cancelled = true; };
     
  }, [period, asOfDate, i18n.language]);

  // P0-2: 加载策略回测统计
  const loadStrategyStats = useCallback(async () => {
    setStrategyStatsLoading(true);
    try {
      const result = await invoke<BacktestComparisonResponse>("backtest_reco_strategies");
      // 按 style 聚合所有 period 的统计
      const byStyle: Record<string, { winRates: number[]; sharpes: number[]; signals: number }> = {};
      for (const [, s] of Object.entries(result.positive.strategies)) {
        const style = s.style;
        if (!byStyle[style]) byStyle[style] = { winRates: [], sharpes: [], signals: 0 };
        byStyle[style].winRates.push(s.winRatePct);
        if (s.sharpeRatio != null) byStyle[style].sharpes.push(s.sharpeRatio);
        byStyle[style].signals += s.totalSignals;
      }
      const agg: Record<string, { winRate: number; sharpe: number | null; signalCount: number }> = {};
      for (const [style, v] of Object.entries(byStyle)) {
        const avgWr = v.winRates.reduce((a, b) => a + b, 0) / v.winRates.length;
        const avgSh = v.sharpes.length > 0 ? v.sharpes.reduce((a, b) => a + b, 0) / v.sharpes.length : null;
        agg[style] = { winRate: Math.round(avgWr * 10) / 10, sharpe: avgSh != null ? Math.round(avgSh * 100) / 100 : null, signalCount: v.signals };
      }
      setStrategyStats(agg);
    } catch {
      // 静默忽略：荐股记录为空或策略回测不可用时 badge 不显示
    }
    setStrategyStatsLoading(false);
  }, []);

  // P0-2: 加载策略回测统计（仅加载一次）
  useEffect(() => {
    loadStrategyStats();
  }, [loadStrategyStats]);

  const handleAnalyze = async (code: string) => {
    await getStockQuote(code);
    await getStockKline(code, "daily", 120);
    startAnalysis(code);
  };

  // P0-1: 批量加载所有 picks 的最近分析结果
  const fetchLatestAnalyses = async (resp: RecoResponse) => {
    const allCodes = new Set<string>();
    for (const arr of Object.values(resp.picks ?? {})) {
      if (!arr) { continue; }
      for (const p of arr) {
        if (!p.synthetic) { allCodes.add(p.stockCode); }
      }
    }
    if (allCodes.size === 0) { return; }

    try {
      const result = await invoke<Record<string, LatestAnalysisSummary | null>>(
        "get_latest_analyses_for_stocks",
        {
          stockCodes: Array.from(allCodes),
          asOfDate,
        },
      );
      setLatestAnalyses(result ?? {});
    } catch (e) {
      console.warn("[RecommendationPanel] Failed to load latest analyses:", e);
      // 降级：显示为空，不影响主流程
    }
  };

  const disabledStyleSet = useMemo(() => new Set(data?.disabledStyles ?? []), [data]);
  const disabledStyleNames = useMemo(() => {
    if (!data) { return ""; }
    return data.disabledStyles?.map((s) => t(`stockAnalysis.recommendation.style${capitalize(s)}`))
      .join(" / ");
  }, [data, t]);

  // B15: degraded styles — as-of 截断导致降级(≠ 缺失),前端用橙色"⛔"标识
  // 与 disabled(灰)区分: disabled 是 vendor 完全不可用;degraded 是可用但效果减弱
  const degradedStyleSet = useMemo(() => new Set(data?.degradedStyles ?? []), [data]);
  const hasDegraded = degradedStyleSet.size > 0;

  // 数据质量统计：所有 picks 总数（包含兜底合成）
  const dataQuality = useMemo(() => {
    if (!data) { return { real: 0, synthetic: 0 }; }
    let real = 0;
    let synthetic = 0;
    for (const arr of Object.values(data.picks ?? {})) {
      if (!arr) { continue; }
      for (const p of arr) {
        if (p.synthetic) { synthetic++; }
        else { real++; }
      }
    }
    return { real, synthetic };
  }, [data]);

  const periodItems = [
    { key: "short", label: t("stockAnalysis.recommendation.periodShort") },
    { key: "mid", label: t("stockAnalysis.recommendation.periodMid") },
    { key: "long", label: t("stockAnalysis.recommendation.periodLong") },
  ];

  const isReplay = anchorMode === "replay" && asOfDate !== null;

  return (
    <Card
      size="small"
      title={
        <div className="flex items-center gap-2">
          <span>{t("stockAnalysis.recommendation.title")}</span>
          {isReplay && <ReplayBadge />}
        </div>
      }
      styles={{ body: { padding: "8px 10px" } }}
      extra={
        <div className="flex items-center gap-2">
          {generatedAtText && (
            <span className="text-[10px] text-gray-400">
              {t("stockAnalysis.recommendation.generatedAt", { time: generatedAtText })}
            </span>
          )}
          <Button size="small" loading={loading} onClick={() => load()}>
            {t("stockAnalysis.settings.panels.refresh")}
          </Button>
        </div>
      }
    >
      <Tabs
        size="small"
        activeKey={period}
        onChange={(k) => setPeriod(k as PeriodKey)}
        items={periodItems}
        style={{ marginBottom: 8 }}
      />

      {isReplay && asOfDate && (
        <Alert
          type="warning"
          showIcon
          className="!text-xs !mb-2"
          message={
            <span className="text-xs">
              {t("timeTravel.recommendationBanner", { date: asOfDate })}
            </span>
          }
        />
      )}

      <div style={{ position: "relative" }}>
        {disabledStyleSet.size > 0 && (
          <Alert
            type="warning"
            showIcon
            className="!text-xs !mb-2"
            message={
              <span className="text-xs">
                {t("stockAnalysis.recommendation.bannerVendorDisabled", { styles: disabledStyleNames })}
              </span>
            }
            action={
              <Button size="small" type="link" onClick={openDataSourceSettings}>
                {t("stockAnalysis.recommendation.openSettings")}
              </Button>
            }
          />
        )}

        {/* 当所有推荐均为兜底合成时提示用户（已自动过滤兜底数据） */}
        {data && dataQuality.real === 0 && dataQuality.synthetic > 0 && (
          <Alert
            type="info"
            showIcon
            className="!text-xs !mb-2"
            message={
              <span className="text-xs">
                {t("stockAnalysis.recommendation.dataQualitySummary", {
                  real: dataQuality.real,
                  synthetic: dataQuality.synthetic,
                })}
              </span>
            }
          />
        )}

        {/* B15: 降级风格提示 —— 与 disabled 不同,degraded 是"可用但效果减弱",用橙色 info 区分 */}
        {data && hasDegraded && asOfDate && (
          <Alert
            type="warning"
            showIcon
            className="!text-xs !mb-2"
            message={
              <span className="text-xs">
                {t("stockAnalysis.recommendation.bannerDegraded", {
                  styles: Array.from(degradedStyleSet)
                    .map((s) => t(`stockAnalysis.recommendation.style${capitalize(s)}`))
                    .join(" / "),
                  date: asOfDate,
                })}
              </span>
            }
          />
        )}

        {loading
          ? <Spin size="small" style={{ display: "block", margin: "16px auto" }} />
          : emptyKind
          ? (
            <PanelEmpty
              kind={emptyKind}
              description={emptyKind === "noData" ? t("stockAnalysis.recommendation.empty") : undefined}
              onOpenSettings={openDataSourceSettings}
            />
          )
          : !data
          ? <Empty image={Empty.PRESENTED_IMAGE_SIMPLE} description={t("stockAnalysis.recommendation.empty")} />
          : (
            // P3-4: key={period} forces the Collapse to remount when period changes,
            // so defaultActiveKey re-applies for the new dataset.
            <Collapse
              key={period}
              ghost
              size="small"
              defaultActiveKey={STYLE_KEYS.filter((s) =>
                !disabledStyleSet.has(s) && (data?.picks?.[s]?.length ?? 0) > 0
              )
                .slice(0, 2)}
              items={STYLE_KEYS.map((style) => {
                const picks = (data?.picks?.[style])?.filter(p => !p.synthetic) ?? [];
                const isDisabled = disabledStyleSet.has(style);
                const isDegraded = degradedStyleSet.has(style);
                // P2-3: when a style is disabled, still show the section (expandable)
                // with a specific empty state explaining why.
                return {
                  key: style,
                  label: (
                    <div className="flex items-center gap-2">
                      <Tag color={STYLE_COLOR[style]} className="m-0 text-xs">
                        {t(`stockAnalysis.recommendation.style${capitalize(style)}`)}
                      </Tag>
                      {/* P0-2: 策略回测徽章 */}
                      {!strategyStatsLoading && strategyStats?.[style] && (
                        <>
                          <Tag
                            className="m-0 text-[10px] leading-4"
                            color={strategyStats[style].winRate >= 55 ? "green" : strategyStats[style].winRate >= 45 ? "orange" : "red"}
                          >
                            {`${strategyStats[style].winRate}%`}
                          </Tag>
                          {strategyStats[style].sharpe != null && (
                            <Tag
                              className="m-0 text-[10px] leading-4"
                              color={strategyStats[style].sharpe! >= 1 ? "green" : strategyStats[style].sharpe! >= 0.5 ? "orange" : "red"}
                            >
                              {`S ${strategyStats[style].sharpe!.toFixed(1)}`}
                            </Tag>
                          )}
                        </>
                      )}
                      {/* B15: 降级风格在 label 处加 ⛔ 徽标(橙色),区别于 disabled 的灰 */}
                      {isDegraded && (
                        <Tag color="orange" className="m-0 text-[10px]">
                          ⛔ {t("timeTravel.degradedStyles.title")}
                        </Tag>
                      )}
                      <span className="text-xs text-gray-500">
                        {isDisabled
                          ? t("stockAnalysis.recommendation.styleDisabled")
                          : `(${picks.length})`}
                      </span>
                    </div>
                  ),
                  children: isDisabled
                    ? (
                      <Empty
                        image={Empty.PRESENTED_IMAGE_SIMPLE}
                        description={t("stockAnalysis.recommendation.styleDisabledReason", {
                          style: t(`stockAnalysis.recommendation.style${capitalize(style)}`),
                        })}
                      />
                    )
                    : picks.length === 0
                    ? (
                      <Empty
                        image={Empty.PRESENTED_IMAGE_SIMPLE}
                        description={t("stockAnalysis.recommendation.empty")}
                      />
                    )
                    : (
                      <List
                        size="small"
                        dataSource={picks}
                        renderItem={(p) => (
                          <PickRow
                            pick={p}
                            onAnalyze={handleAnalyze}
                            latestAnalysis={latestAnalyses[p.stockCode] ?? null}
                          />
                        )}
                      />
                    ),
                };
              })}
            />
          )}
        {isReplay && <ReplayWatermark />}
      </div>
    </Card>
  );
}

function capitalize(s: string) {
  return s.charAt(0).toUpperCase() + s.slice(1);
}

/** 荐股 ↔ 分析师共识交叉验证徽章
 *  - 推荐为 BUY，仅在有缓存共识时显示（避免噪音）
 *  - 共识看多 → 绿色 ✓
 *  - 共识看空 / 中性 / 分歧 → 警示色 ⚠
 */
function CrossCheckBadge({
  consensus,
  recAction,
}: {
  consensus: StockConsensus;
  recAction: string;
}) {
  const { t, i18n } = useTranslation();
  if (consensus.total === 0) { return null; }

  // 推荐与共识的对齐：BUY 时要求共识 bullish 才算"一致"；
  // 其他动作（HOLD/SELL）暂不参与交叉验证，留给后续扩展。
  const aligned = recAction === "BUY" ? consensus.consensus === "bullish" : null;

  let color: "green" | "red" | "orange" | "gold";
  let icon: string;
  let label: string;
  let tooltipBody: string;

  if (aligned === true) {
    color = "green";
    icon = "✓";
    label = t("stockAnalysis.recommendation.consensusBullish");
    tooltipBody = t("stockAnalysis.recommendation.crossCheckAligned");
  } else if (consensus.consensus === "bearish") {
    color = "red";
    icon = "⚠";
    label = t("stockAnalysis.recommendation.consensusBearish");
    tooltipBody = t("stockAnalysis.recommendation.crossCheckBearish");
  } else if (consensus.consensus === "divided") {
    color = "gold";
    icon = "⚠";
    label = t("stockAnalysis.recommendation.consensusDivided");
    tooltipBody = t("stockAnalysis.recommendation.crossCheckDivided");
  } else {
    // 共识中性，且推荐为 BUY → 直接提示"未印证 BUY 推荐"
    color = "orange";
    icon = "⚠";
    label = `${consensus.neutral}/${consensus.total} ${t("stockAnalysis.recommendation.neutral")}`;
    tooltipBody = t("stockAnalysis.recommendation.crossCheckNeutral", {
      total: consensus.total,
      neutral: consensus.neutral,
    });
  }

  const updatedAtText = new Date(consensus.updatedAt).toLocaleString(
    i18n.language === "zh-CN" ? "zh-CN" : "en-US",
    { month: "2-digit", day: "2-digit", hour: "2-digit", minute: "2-digit" },
  );

  return (
    <Tooltip
      title={
        <div className="text-[11px] space-y-0.5">
          <div>{tooltipBody}</div>
          <div style={{ opacity: 0.75 }}>
            {t("stockAnalysis.recommendation.crossCheckTitle", { updatedAt: updatedAtText })}
          </div>
        </div>
      }
    >
      <Tag color={color} className="m-0 text-[10px]">
        {icon} {label}
      </Tag>
    </Tooltip>
  );
}

function PickRow(
  { pick, onAnalyze, latestAnalysis }: {
    pick: RecoPick;
    onAnalyze: (code: string) => void;
    latestAnalysis: LatestAnalysisSummary | null;
  },
) {
  const { t } = useTranslation();
  // 读荐股 ↔ 分析师交叉验证缓存（仅当该股已有最近一次工作流结果时存在）
  const stockCodeConsensus = useStockAnalysisStore((s) => s.stockCodeConsensus);
  const consensus = stockCodeConsensus[pick.stockCode];

  // P0-1: 上次分析结论的视觉展示
  const historyBadge = useMemo(() => {
    if (!latestAnalysis || latestAnalysis.status !== "completed") { return null; }
    const action = parseAction(latestAnalysis.decisionAction);
    let color: string;
    let label: string;
    switch (action) {
      case "BUY":
      case "INCREASE":
        color = "red";
        label = t("stockAnalysis.actionBuy");
        break;
      case "SELL":
      case "REDUCE":
        color = "green";
        label = t("stockAnalysis.actionSell");
        break;
      case "UNCERTAIN":
        color = "default";
        label = t("stockAnalysis.actionUncertain");
        break;
      default:
        color = "blue";
        label = t("stockAnalysis.actionHold");
    }
    const confText = latestAnalysis.confidence != null ? ` ${latestAnalysis.confidence}` : "";

    return (
      <Tooltip
        title={
          <div className="text-[11px] space-y-0.5">
            <div>
              {t("stockAnalysis.recommendation.lastAnalysis", {
                date: latestAnalysis.analysisDate,
                action: label,
                confidence: confText,
              })}
            </div>
            {latestAnalysis.outcome && latestAnalysis.outcome !== "pending" && (
              <div>
                {t("stockAnalysis.recommendation.outcome")}: {latestAnalysis.outcome === "win"
                  ? t("stockAnalysis.recommendation.outcomeWin")
                  : t("stockAnalysis.recommendation.outcomeLoss")}
              </div>
            )}
          </div>
        }
      >
        <Tag color={color} className="m-0 text-[10px]" style={{ opacity: 0.8 }}>
          {label}
          {confText}
          {latestAnalysis.outcome === "win" && " ✓"}
          {latestAnalysis.outcome === "loss" && " ✗"}
        </Tag>
      </Tooltip>
    );
  }, [latestAnalysis, t]);
  const content = (
    <div className="text-xs w-full flex flex-col gap-0.5 py-0.5">
      <div className="flex items-center gap-1.5">
        <Tag className="m-0 text-[10px]">{pick.stockCode}</Tag>
        <span className="font-medium truncate flex-1">{pick.stockName}</span>
        <Tag color="volcano" className="m-0 text-[10px]">BUY</Tag>
        {historyBadge}
        {consensus && <CrossCheckBadge consensus={consensus} recAction="BUY" />}
        {pick.synthetic
          ? (
            <Tooltip title={t("stockAnalysis.recommendation.syntheticTooltip")}>
              <Tag color="orange" className="m-0 text-[10px]">
                {t("stockAnalysis.recommendation.tagSynthetic")}
              </Tag>
            </Tooltip>
          )
          : (
            <Tooltip title={t("stockAnalysis.recommendation.realTooltip")}>
              <Tag color="green" className="m-0 text-[10px]">
                {t("stockAnalysis.recommendation.tagReal")}
              </Tag>
            </Tooltip>
          )}
        <span className="font-mono text-[10px] text-gray-500">{fmt(pick.price)}</span>
      </div>
      <div className="flex items-center gap-1.5 text-[10px] text-gray-500">
        <span>
          {t("stockAnalysis.recommendation.row.entry")} {fmt(pick.entryLow)}-{fmt(pick.entryHigh)}
        </span>
        <span className="text-red-500">
          {t("stockAnalysis.recommendation.row.stopLoss")} {fmt(pick.stopLoss)}
        </span>
        <span className="text-green-500">
          {t("stockAnalysis.recommendation.row.target")} {fmt(pick.targetPrice)}
        </span>
      </div>
      <div className="flex items-center gap-1.5 text-[10px] text-gray-500">
        <span>
          {t("stockAnalysis.recommendation.row.position")} {fmt(pick.positionPct, 1)}%
        </span>
        <span>
          {t("stockAnalysis.recommendation.row.holding")} {fmt(pick.holdingDays, 0, fmt(0, 0))}d
        </span>
        <Tag color="blue" className="m-0 text-[10px]">
          {t("stockAnalysis.recommendation.row.confidence")} {fmt(pick.confidence, 0, "0")}
        </Tag>
        {pick.secondaryStyles && pick.secondaryStyles.length > 0 && (
          <span className="text-gray-400">
            ({t("stockAnalysis.recommendation.row.secondaryStyle")}:
            {pick.secondaryStyles?.map((s) => t(`stockAnalysis.recommendation.style${capitalize(s)}`)).join("/")})
          </span>
        )}
      </div>
    </div>
  );
  return (
    <Tooltip
      title={
        <div className="text-xs">
          <div className="font-medium mb-1">{pick.stockName} ({pick.stockCode})</div>
          {pick.reasons.length > 0 && (
            <div className="mb-1">
              {/* P1-1: i18n for "Reasons" label */}
              <div className="text-green-600">
                {t("stockAnalysis.recommendation.row.reasons")}：
              </div>
              <ul className="m-0 pl-4">{pick.reasons?.map((r, i) => <li key={i}>{r}</li>)}</ul>
            </div>
          )}
          {pick.riskNotes.length > 0 && (
            <div>
              <div className="text-red-600">
                {t("stockAnalysis.recommendation.row.risks")}：
              </div>
              <ul className="m-0 pl-4">{pick.riskNotes?.map((r, i) => <li key={i}>{r}</li>)}</ul>
            </div>
          )}
        </div>
      }
    >
      <List.Item
        style={{ cursor: "pointer", padding: "4px 0" }}
        onClick={() => onAnalyze(pick.stockCode)}
      >
        {content}
      </List.Item>
    </Tooltip>
  );
}
