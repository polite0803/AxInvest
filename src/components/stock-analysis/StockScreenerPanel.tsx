import { invoke } from "@/lib/invoke";
import { useStockAnalysisStore } from "@/stores";
import { SearchOutlined } from "@ant-design/icons";
import { Button, Card, Empty, InputNumber, List, Space, Spin, Tag } from "antd";
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
  { key: "minChangePct", label: "涨跌幅≥", unit: "%", min: -10, max: 10, step: 0.5 },
  { key: "turnoverRateMin", label: "换手率≥", unit: "%", min: 0, max: 50, step: 0.5 },
  { key: "mainInflowMin", label: "主力净流入≥", unit: "万元", min: 0, max: 999999, step: 100 },
  { key: "dragonTigerNetMin", label: "龙虎榜净买≥", unit: "万元", min: 0, max: 999999, step: 100 },
  { key: "northboundRatioMin", label: "北向持仓≥", unit: "%", min: 0, max: 100, step: 0.5 },
  { key: "rsiOversold", label: "RSI 超卖", unit: "", min: 0, max: 0, step: 0 },
  { key: "rsiOverbought", label: "RSI 超买", unit: "", min: 0, max: 0, step: 0 },
] as const;

export function StockScreenerPanel() {
  const { t } = useTranslation();
  const getStockQuote = useStockAnalysisStore((s) => s.getStockQuote);
  const getStockKline = useStockAnalysisStore((s) => s.getStockKline);
  const startAnalysis = useStockAnalysisStore((s) => s.startAnalysis);
  const watchlistVersion = useStockAnalysisStore((s) => s.watchlistVersion);

  const [results, setResults] = useState<ScreenResult[]>([]);
  const [loading, setLoading] = useState(false);
  const [mode, setMode] = useState<"discover" | "screen">("discover");
  const [factors, setFactors] = useState<Record<string, FactorState>>({});
  const [selectedCount, setSelectedCount] = useState(0);

  const discover = useCallback(async () => {
    setLoading(true);
    try {
      const r = await invoke<ScreenResult[]>("discover_stock_candidates");
      if (Array.isArray(r)) { setResults(r); }
    } catch { /* 静默 */ }
    setLoading(false);
  }, []);

  const screen = useCallback(async () => {
    setLoading(true);
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
    } catch { /* 静默 */ }
    setLoading(false);
  }, [factors]);

  useEffect(() => {
    discover();
  }, [watchlistVersion, discover]);

  const toggleFactor = (key: string) => {
    setFactors((prev) => {
      const cur = prev[key];
      const enabled = !cur?.enabled;
      const next = { ...prev, [key]: { ...cur, enabled, value: cur?.value } };
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
              if (await checkVendorEnabled("screener")) { discover(); }
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
          <div className="flex flex-wrap gap-1.5">
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
                  {fd.label}
                </Tag>
              );
            })}
          </div>
          {selectedCount > 0 && (
            <div className="flex gap-1 flex-wrap items-center text-xs mt-1">
              {FACTOR_DEFS.filter((fd) => factors[fd.key]?.enabled).map((fd) => {
                if (isRsiFactor(fd.key)) { return null; }
                return (
                  <Space key={fd.key} size={2}>
                    <span className="text-gray-500">{fd.label}</span>
                    <InputNumber
                      size="small"
                      style={{ width: 72 }}
                      min={fd.min}
                      max={fd.max}
                      step={fd.step}
                      value={factors[fd.key]?.value}
                      onChange={(v) => setValue(fd.key, v)}
                      placeholder={fd.unit || t("stockAnalysis.settings.screener.placeholder")}
                      suffix={fd.unit || undefined}
                    />
                  </Space>
                );
              })}
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
          )}
        </div>
      )}

      {loading
        ? <Spin size="small" />
        : results.length === 0
        ? (
          <Empty
            image={Empty.PRESENTED_IMAGE_SIMPLE}
            description={mode === "discover"
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
