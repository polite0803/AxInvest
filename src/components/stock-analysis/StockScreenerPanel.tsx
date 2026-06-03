import { List } from "@/components/common/AntdList";
import { invoke } from "@/lib/invoke";
import { useStockAnalysisStore } from "@/stores";
import { SearchOutlined } from "@ant-design/icons";
import { Button, Card, Empty, InputNumber, Spin, Tag } from "antd";
import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { checkVendorEnabled } from "./vendorCheck";

interface ScreenResult {
  stockCode: string;
  stockName: string;
  price: number;
  changePct: number;
  reasons: string[];
  score: number;
}

interface FactorState {
  enabled: boolean;
  value?: number;
}

const FACTOR_DEFS = [
  {
    key: "minChangePct",
    i18nKey: "stockAnalysis.settings.screener.factor.changePct",
    unit: "%",
    min: -10,
    max: 10,
    step: 0.5,
    default: 3,
  },
  {
    key: "turnoverRateMin",
    i18nKey: "stockAnalysis.settings.screener.factor.turnover",
    unit: "%",
    min: 0,
    max: 50,
    step: 0.5,
    default: 3,
  },
  {
    key: "mainInflowMin",
    i18nKey: "stockAnalysis.settings.screener.factor.mainInflow",
    unit: "万元",
    unitI18n: true as const,
    min: 0,
    max: 999999,
    step: 100,
    default: 1000,
  },
  {
    key: "dragonTigerNetMin",
    i18nKey: "stockAnalysis.settings.screener.factor.dragonTiger",
    unit: "万元",
    unitI18n: true as const,
    min: 0,
    max: 999999,
    step: 100,
    default: 500,
  },
  {
    key: "northboundRatioMin",
    i18nKey: "stockAnalysis.settings.screener.factor.northbound",
    unit: "%",
    min: 0,
    max: 100,
    step: 0.5,
    default: 1,
  },
  {
    key: "rsiOversold",
    i18nKey: "stockAnalysis.settings.screener.factor.rsiOversold",
    unit: "",
    min: 0,
    max: 0,
    step: 0,
  },
  {
    key: "rsiOverbought",
    i18nKey: "stockAnalysis.settings.screener.factor.rsiOverbought",
    unit: "",
    min: 0,
    max: 0,
    step: 0,
  },
] as const;

export function StockScreenerPanel() {
  const { t } = useTranslation();
  const getStockQuote = useStockAnalysisStore((s) => s.getStockQuote);
  const getStockKline = useStockAnalysisStore((s) => s.getStockKline);
  const startAnalysis = useStockAnalysisStore((s) => s.startAnalysis);
  const watchlistVersion = useStockAnalysisStore((s) => s.watchlistVersion);

  const [results, setResults] = useState<ScreenResult[]>([]);
  const [loading, setLoading] = useState(false);
  const [fetchError, setFetchError] = useState(false);
  const [mode, setMode] = useState<"discover" | "screen">("discover");
  const [factors, setFactors] = useState<Record<string, FactorState>>({});
  const [selectedCount, setSelectedCount] = useState(0);

  const discover = useCallback(async () => {
    setLoading(true);
    setFetchError(false);
    try {
      const r = await invoke<ScreenResult[]>("discover_stock_candidates");
      if (Array.isArray(r)) { setResults(r); }
    } catch {
      setFetchError(true);
    }
    setLoading(false);
  }, []);

  const screen = useCallback(async () => {
    setLoading(true);
    setFetchError(false);
    try {
      const criteria: Record<string, any> = {};
      for (const fd of FACTOR_DEFS) {
        const f = factors[fd.key];
        if (!f?.enabled) { continue; }
        if (fd.key === "rsiOversold") { criteria.rsiOversold = true; }
        else if (fd.key === "rsiOverbought") { criteria.rsiOverbought = true; }
        else if (f.value != null) { criteria[fd.key] = f.value; }
      }
      const r = await invoke<ScreenResult[]>("screen_stocks", { criteria });
      if (Array.isArray(r)) { setResults(r); }
    } catch {
      setFetchError(true);
    }
    setLoading(false);
  }, [factors]);

  useEffect(() => {
    discover();
  }, [watchlistVersion, discover]);

  const toggleFactor = (key: string) => {
    setFactors((prev) => {
      const cur = prev[key];
      const enabled = !cur?.enabled;
      const fd = FACTOR_DEFS.find((f) => f.key === key);
      const value = enabled ? (cur?.value ?? ("default" in fd! ? (fd as any).default : undefined)) : cur?.value;
      const next = { ...prev, [key]: { ...cur, enabled, value } };
      setSelectedCount(Object.values(next).filter((f) => f.enabled).length);
      return next;
    });
  };

  const setValue = (key: string, v: number | null) => {
    setFactors((prev) => ({ ...prev, [key]: { ...prev[key], value: v ?? undefined } }));
  };

  const handleAnalyze = async (code: string) => {
    await getStockQuote(code);
    await getStockKline(code, "daily", 120);
    startAnalysis(code);
  };

  const isRsiFactor = (key: string) => key.startsWith("rsi");

  return (
    <Card
      size="small"
      title={t("stockAnalysis.settings.screener.title")}
      styles={{ body: { padding: "8px 10px" } }}
      extra={
        <div className="flex gap-1">
          <Button
            size="small"
            type={mode === "discover" ? "primary" : "default"}
            onClick={async () => {
              setMode("discover");
              const r = await checkVendorEnabled("screener");
              if (r.status === "ok") { discover(); }
            }}
          >
            {t("stockAnalysis.settings.screener.discover")}
          </Button>
          <Button size="small" type={mode === "screen" ? "primary" : "default"} onClick={() => setMode("screen")}>
            {t("stockAnalysis.settings.screener.screen")}
          </Button>
        </div>
      }
    >
      {mode === "screen" && (
        <div className="flex flex-col gap-1 mb-2">
          <div className="text-xs text-gray-400 mb-1">
            {t("stockAnalysis.settings.screener.factorHint", { count: selectedCount })}
          </div>
          <div className="flex flex-wrap gap-0.5">
            {FACTOR_DEFS.map((fd) => {
              const f = factors[fd.key];
              const active = f?.enabled;
              return (
                <Tag
                  key={fd.key}
                  color={active ? "blue" : "default"}
                  className="cursor-pointer text-xs m-0 select-none"
                  onClick={() => toggleFactor(fd.key)}
                >
                  {active ? "✓ " : ""}
                  {t(fd.i18nKey)}
                </Tag>
              );
            })}
          </div>
          {selectedCount > 0 && (
            <div className="flex flex-col gap-1 items-stretch text-xs mt-1">
              {FACTOR_DEFS.filter((fd) => factors[fd.key]?.enabled).map((fd) => {
                if (isRsiFactor(fd.key)) { return null; }
                return (
                  <div key={fd.key} className="flex items-center gap-1">
                    <span className="text-gray-500 shrink-0">{t(fd.i18nKey)}</span>
                    <InputNumber
                      size="small"
                      style={{ flex: 1, minWidth: 50 }}
                      min={fd.min}
                      max={fd.max}
                      step={fd.step}
                      value={factors[fd.key]?.value}
                      onChange={(v) => setValue(fd.key, v)}
                      placeholder={fd.unit || t("stockAnalysis.settings.screener.placeholder")}
                      suffix={"unitI18n" in fd ? t("stockAnalysis.settings.screener.unit10k") : fd.unit || undefined}
                    />
                  </div>
                );
              })}
              <div className="flex gap-1">
                <Button size="small" icon={<SearchOutlined />} onClick={screen} loading={loading} type="primary">
                  {t("stockAnalysis.settings.screener.filter")}
                </Button>
                <Button
                  size="small"
                  onClick={() => {
                    setFactors({});
                    setSelectedCount(0);
                  }}
                >
                  {t("stockAnalysis.settings.screener.clear")}
                </Button>
              </div>
            </div>
          )}
        </div>
      )}

      {loading
        ? <Spin size="small" />
        : results.length === 0
        ? (
          <Empty
            image={Empty.PRESENTED_IMAGE_SIMPLE}
            description={fetchError ? t("stockAnalysis.settings.vendor.connectionFailed") : mode === "discover"
              ? t("stockAnalysis.settings.screener.discoverHint")
              : t("stockAnalysis.settings.screener.screenHint")}
          />
        )
        : (
          <List
            size="small"
            dataSource={results.slice(0, 15)}
            renderItem={(r) => (
              <List.Item
                style={{ cursor: "pointer", padding: "4px 0" }}
                onClick={() => handleAnalyze(r.stockCode)}
                actions={[
                  <Tag key="score" color="blue" className="text-xs m-0">
                    {t("stockAnalysis.settings.screener.score", { score: r.score })}
                  </Tag>,
                ]}
              >
                <div className="flex items-center gap-2 text-xs w-full">
                  <Tag className="m-0 text-xs">{r.stockCode}</Tag>
                  <span className="flex-1 truncate">{r.stockName}</span>
                  <span className="font-mono">{r.price.toFixed(2)}</span>
                  <span className={r.changePct >= 0 ? "text-red-500" : "text-green-500"}>
                    {r.changePct >= 0 ? "+" : ""}
                    {r.changePct.toFixed(2)}%
                  </span>
                  {r.reasons.slice(0, 2).map((reason, i) => (
                    <Tag key={i} color="green" className="text-xs m-0">{reason}</Tag>
                  ))}
                </div>
              </List.Item>
            )}
          />
        )}
    </Card>
  );
}
