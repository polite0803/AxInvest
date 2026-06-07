import { List } from "@/components/common/AntdList";
import { invoke } from "@/lib/invoke";
import { useStockAnalysisStore } from "@/stores";
import type { StockConsensus } from "@/types";
import { Alert, Button, Card, Collapse, Empty, Spin, Tabs, Tag, Tooltip } from "antd";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { PanelEmpty, type PanelEmptyKind } from "./PanelEmpty";
import { useStockAnalysisPage } from "./StockAnalysisPageContext";

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
  generatedAt: number;
  rawSeedPoolSize: number;
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

export function RecommendationPanel() {
  const { t, i18n } = useTranslation();
  const { openDataSourceSettings } = useStockAnalysisPage();
  const getStockQuote = useStockAnalysisStore((s) => s.getStockQuote);
  const getStockKline = useStockAnalysisStore((s) => s.getStockKline);
  const startAnalysis = useStockAnalysisStore((s) => s.startAnalysis);

  const [period, setPeriod] = useState<PeriodKey>("short");
  const [data, setData] = useState<RecoResponse | null>(null);
  const [loading, setLoading] = useState(false);
  const [emptyKind, setEmptyKind] = useState<PanelEmptyKind | null>(null);
  const [generatedAtText, setGeneratedAtText] = useState<string>("");

  // P1-2: monotonically increasing request token — discard stale results.
  const reqTokenRef = useRef(0);

  const load = useCallback(
    async () => {
      const myToken = ++reqTokenRef.current;
      setLoading(true);
      setEmptyKind(null);
      try {
        // 后端会基于 FALLBACK_STOCKS 兜底 seed pool，并在响应里返回 disabledStyles；
        // 这里不再做硬性 vendor 门控（否则会卡死整个面板）。
        const r = await invoke<RecoResponse>("recommend_stocks", { period });
        if (myToken !== reqTokenRef.current) { return; // stale
         }
        if (!r || !r.picks || Object.keys(r.picks).length === 0) {
          setData(r ?? null);
          // 区分"全风格 disabled" vs "有 enabled 但没出 picks"
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
      } catch (e: any) {
        // P3-2: don't swallow the error
        // eslint-disable-next-line no-console
        console.error("[RecommendationPanel] load failed:", e);
        if (myToken !== reqTokenRef.current) { return; }
        setData(null);
        setEmptyKind("connectionFailed");
      }
      if (myToken === reqTokenRef.current) { setLoading(false); }
    },
    [period, i18n.language],
  );

  useEffect(() => {
    load();
  }, [load]);

  const handleAnalyze = async (code: string) => {
    await getStockQuote(code);
    await getStockKline(code, "daily", 120);
    startAnalysis(code);
  };

  const disabledStyleSet = useMemo(() => new Set(data?.disabledStyles ?? []), [data]);
  const disabledStyleNames = useMemo(() => {
    if (!data) { return ""; }
    return data.disabledStyles
      .map((s) => t(`stockAnalysis.recommendation.style${capitalize(s)}`))
      .join(" / ");
  }, [data, t]);

  // 真实 / 兜底 picks 统计：用于顶部 banner 提示用户当前数据是真实策略命中
  // 还是数据稀疏兜底合成（vendor K 线 / 财务 / 资金不可用时）。
  const dataQuality = useMemo(() => {
    if (!data) { return { real: 0, synthetic: 0 }; }
    let real = 0;
    let synthetic = 0;
    for (const arr of Object.values(data.picks)) {
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

  return (
    <Card
      size="small"
      title={t("stockAnalysis.recommendation.title")}
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

      {/* 数据质量提示：当存在兜底 picks 时提示用户当前数据稀疏 */}
      {data && dataQuality.synthetic > 0 && (
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
            defaultActiveKey={STYLE_KEYS.filter((s) => !disabledStyleSet.has(s) && (data.picks[s]?.length ?? 0) > 0)
              .slice(0, 2)}
            items={STYLE_KEYS.map((style) => {
              const picks = data.picks[style] ?? [];
              const isDisabled = disabledStyleSet.has(style);
              // P2-3: when a style is disabled, still show the section (expandable)
              // with a specific empty state explaining why.
              return {
                key: style,
                label: (
                  <div className="flex items-center gap-2">
                    <Tag color={STYLE_COLOR[style]} className="m-0 text-xs">
                      {t(`stockAnalysis.recommendation.style${capitalize(style)}`)}
                    </Tag>
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
                  ? <Empty image={Empty.PRESENTED_IMAGE_SIMPLE} description={t("stockAnalysis.recommendation.empty")} />
                  : (
                    <List
                      size="small"
                      dataSource={picks}
                      renderItem={(p) => <PickRow pick={p} onAnalyze={handleAnalyze} />}
                    />
                  ),
              };
            })}
          />
        )}
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

function PickRow({ pick, onAnalyze }: { pick: RecoPick; onAnalyze: (code: string) => void }) {
  const { t } = useTranslation();
  // 读荐股 ↔ 分析师交叉验证缓存（仅当该股已有最近一次工作流结果时存在）
  const stockCodeConsensus = useStockAnalysisStore((s) => s.stockCodeConsensus);
  const consensus = stockCodeConsensus[pick.stockCode];
  const content = (
    <div className="text-xs w-full flex flex-col gap-0.5 py-0.5">
      <div className="flex items-center gap-1.5">
        <Tag className="m-0 text-[10px]">{pick.stockCode}</Tag>
        <span className="font-medium truncate flex-1">{pick.stockName}</span>
        <Tag color="volcano" className="m-0 text-[10px]">BUY</Tag>
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
            {pick.secondaryStyles.map((s) => t(`stockAnalysis.recommendation.style${capitalize(s)}`)).join("/")})
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
              <ul className="m-0 pl-4">{pick.reasons.map((r, i) => <li key={i}>{r}</li>)}</ul>
            </div>
          )}
          {pick.riskNotes.length > 0 && (
            <div>
              <div className="text-red-600">
                {t("stockAnalysis.recommendation.row.risks")}：
              </div>
              <ul className="m-0 pl-4">{pick.riskNotes.map((r, i) => <li key={i}>{r}</li>)}</ul>
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
