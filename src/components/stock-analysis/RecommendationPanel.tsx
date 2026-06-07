import { List } from "@/components/common/AntdList";
import { invoke } from "@/lib/invoke";
import { useStockAnalysisStore } from "@/stores";
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

function PickRow({ pick, onAnalyze }: { pick: RecoPick; onAnalyze: (code: string) => void }) {
  const { t } = useTranslation();
  const content = (
    <div className="text-xs w-full flex flex-col gap-0.5 py-0.5">
      <div className="flex items-center gap-1.5">
        <Tag className="m-0 text-[10px]">{pick.stockCode}</Tag>
        <span className="font-medium truncate flex-1">{pick.stockName}</span>
        <Tag color="volcano" className="m-0 text-[10px]">BUY</Tag>
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
