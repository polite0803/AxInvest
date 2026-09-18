import { invoke } from "@/lib/invoke";
import type { QuantSimResult } from "@/types/market-sim";
import { Button, Card, Descriptions, Input, InputNumber, Select, Spin, Statistic, Tag } from "antd";
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

interface QuantSimPanelProps {
  /** 当前分析标的代码 —— 作为面板默认值，用户仍可覆盖 */
  stockCode?: string;
  /** 当前价（元）—— 面板内部换算为「分」提交后端 */
  referencePriceYuan?: number | null;
}

const FALLBACK_STOCK_CODE = "000001";
const FALLBACK_REF_PRICE_FEN = 1000;

/**
 * QuantSimPanel — 策略沙盒：在模拟市场里跑一条单策略并观察市场活跃度。
 *
 * 作为股票分析页「模拟仿真」标签的子面板，标的默认取自当前分析上下文。
 * 注意它输出的是**市场级活跃度**（事件数 / 成交数 / 终价），
 * 不是该策略在这只股票上的收益归因。
 */
export function QuantSimPanel({ stockCode: stockCodeProp, referencePriceYuan }: QuantSimPanelProps = {}) {
  const { t } = useTranslation();

  const STRATEGIES = [
    { value: "ma_cross", label: t("quant.strategySelect.maCross") },
    { value: "macd", label: t("quant.strategySelect.macd") },
    { value: "rsi", label: t("quant.strategySelect.rsi") },
    { value: "boll", label: t("quant.strategySelect.boll") },
    { value: "turtle", label: t("quant.strategySelect.turtle") },
  ];
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [result, setResult] = useState<QuantSimResult | null>(null);
  const [stockCode, setStockCode] = useState(() => stockCodeProp?.trim() || FALLBACK_STOCK_CODE);
  const [refPrice, setRefPrice] = useState(() =>
    referencePriceYuan != null && referencePriceYuan > 0
      ? Math.round(referencePriceYuan * 100)
      : FALLBACK_REF_PRICE_FEN
  );
  const [simMs, setSimMs] = useState(500);
  const [strategy, setStrategy] = useState("ma_cross");
  const tokenRef = useRef(0);
  const mountedRef = useRef(false);

  // 上下文可能在挂载后才到（行情异步加载）⇒ 变化时同步；首次由 useState 处理
  useEffect(() => {
    if (!mountedRef.current) {
      mountedRef.current = true;
      return;
    }
    if (stockCodeProp?.trim()) {
      setStockCode(stockCodeProp.trim());
    }
  }, [stockCodeProp]);

  useEffect(() => {
    if (referencePriceYuan != null && referencePriceYuan > 0) {
      setRefPrice(Math.round(referencePriceYuan * 100));
    }
  }, [referencePriceYuan]);

  const refPriceYuanText = refPrice > 0 ? (refPrice / 100).toFixed(2) : null;

  const handleRun = async () => {
    const myToken = ++tokenRef.current;
    setLoading(true);
    setError(null);
    setResult(null);

    try {
      const res = await invoke<QuantSimResult>("market_sim_run_strategy", {
        request: {
          stockCode,
          referencePrice: refPrice,
          strategyName: strategy,
          maxSimTimeMs: simMs,
        },
      });
      if (myToken !== tokenRef.current) {
        return;
      }
      setResult(res);
    } catch (e: unknown) {
      if (myToken !== tokenRef.current) {
        return;
      }
      setError(typeof e === "string" ? e : e instanceof Error ? e.message : String(e));
    } finally {
      if (myToken === tokenRef.current) {
        setLoading(false);
      }
    }
  };

  return (
    <div className="space-y-4">
      <Card size="small" title={t("stockAnalysis.quantSim.title")}>
        <div className="mb-3 flex flex-wrap items-center gap-4">
          <label className="text-sm font-medium">
            {t("stockAnalysis.quantSim.stockCode")}
            {/* 股票代码是标识符：用文本输入，避免 InputNumber 把 000001 读成 1 */}
            <Input
              className="ml-2"
              style={{ width: 110 }}
              maxLength={6}
              value={stockCode}
              onChange={(e) => setStockCode(e.target.value)}
            />
          </label>
          <label className="text-sm font-medium">
            {t("stockAnalysis.quantSim.referencePrice")}
            <InputNumber
              className="ml-2"
              style={{ width: 120 }}
              min={1}
              value={refPrice}
              onChange={(v) => setRefPrice(v ?? FALLBACK_REF_PRICE_FEN)}
            />
            {refPriceYuanText && (
              <span className="ml-2 text-xs text-secondary">
                {t("stockAnalysis.simulation.yuanEquivalent", { yuan: refPriceYuanText })}
              </span>
            )}
          </label>
          <label className="text-sm font-medium">
            {t("stockAnalysis.quantSim.duration")}
            <InputNumber
              className="ml-2"
              style={{ width: 100 }}
              min={1}
              max={5000}
              value={simMs}
              onChange={(v) => setSimMs(v ?? 500)}
            />
          </label>
        </div>

        <div className="mb-3 flex items-center gap-4">
          <label className="text-sm font-medium">{t("stockAnalysis.quantSim.strategy")}</label>
          <Select
            style={{ width: 240 }}
            value={strategy}
            onChange={setStrategy}
            options={STRATEGIES}
          />
          <Button type="primary" onClick={handleRun} loading={loading}>
            {loading ? t("stockAnalysis.quantSim.running") : t("stockAnalysis.quantSim.run")}
          </Button>
        </div>

        <div className="text-xs text-secondary">
          {t("stockAnalysis.quantSim.description")}
        </div>
      </Card>

      {loading && (
        <Card size="small">
          <div className="flex items-center justify-center py-6">
            <Spin description={t("stockAnalysis.quantSim.spinTip")} />
          </div>
        </Card>
      )}

      {error && (
        <Card size="small">
          <div className="py-3 text-center text-red">{error}</div>
        </Card>
      )}

      {result && !loading && (
        <>
          <div className="grid grid-cols-4 gap-3">
            <Card size="small" hoverable>
              <Statistic
                title={t("stockAnalysis.quantSim.events")}
                value={result.totalEvents}
                suffix={t("stockAnalysis.quantSim.eventsSuffix")}
              />
            </Card>
            <Card size="small" hoverable>
              <Statistic
                title={t("stockAnalysis.quantSim.trades")}
                value={result.totalTrades}
                suffix={t("stockAnalysis.quantSim.tradesSuffix")}
              />
            </Card>
            <Card size="small" hoverable>
              <Statistic
                title={t("stockAnalysis.quantSim.finalPrice")}
                value={result.finalMidPrice ?? "—"}
                suffix={t("stockAnalysis.quantSim.fenSuffix")}
              />
            </Card>
            <Card size="small" hoverable>
              <Statistic title={t("stockAnalysis.quantSim.wallClock")} value={result.wallClockMs} suffix="ms" />
            </Card>
          </div>

          <Card size="small" title={t("stockAnalysis.quantSim.interpretation")}>
            <Descriptions column={1} size="small">
              <Descriptions.Item label={t("stockAnalysis.quantSim.strategyLabel")}>
                <Tag color="blue">
                  {STRATEGIES.find((s) => s.value === strategy)?.label ?? strategy}
                </Tag>
              </Descriptions.Item>
              <Descriptions.Item label={t("stockAnalysis.quantSim.marketActivity")}>
                {result.totalEvents > 0
                  ? t("stockAnalysis.quantSim.marketActivityDetail", {
                    simMs,
                    totalEvents: result.totalEvents,
                    totalTrades: result.totalTrades,
                  })
                  : t("stockAnalysis.quantSim.noEvents")}
              </Descriptions.Item>
              <Descriptions.Item label={t("stockAnalysis.quantSim.quote")}>
                {result.finalMidPrice
                  ? t("stockAnalysis.quantSim.quoteDetail", { finalMidPrice: result.finalMidPrice, refPrice })
                  : t("stockAnalysis.quantSim.noQuote")}
              </Descriptions.Item>
            </Descriptions>
          </Card>
        </>
      )}

      {!result && !loading && !error && (
        <Card size="small">
          <div className="py-6 text-center text-secondary">
            <p className="mb-1 text-base">{t("stockAnalysis.quantSim.emptyHint")}</p>
            <p className="text-sm">
              {t("stockAnalysis.quantSim.emptyDesc")}
            </p>
          </div>
        </Card>
      )}
    </div>
  );
}
