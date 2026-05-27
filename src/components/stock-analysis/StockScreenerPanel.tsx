import { invoke } from "@/lib/invoke";
import { useStockAnalysisStore } from "@/stores";
import { SearchOutlined } from "@ant-design/icons";
import { Button, Card, Checkbox, Empty, InputNumber, List, Spin, Tag } from "antd";
import { useEffect, useState } from "react";

interface ScreenResult {
  stockCode: string;
  stockName: string;
  price: number;
  changePct: number;
  reasons: string[];
  score: number;
}

interface CriteriaState {
  minChangePct?: number;
  maxChangePct?: number;
  turnoverRateMin?: number;
  dragonTigerNetMin?: number;
  mainInflowMin?: number;
  northboundRatioMin?: number;
  rsiOversold: boolean;
  rsiOverbought: boolean;
}

const defaultCriteria: CriteriaState = {
  rsiOversold: false,
  rsiOverbought: false,
};

export function StockScreenerPanel() {
  const getStockQuote = useStockAnalysisStore((s) => s.getStockQuote);
  const getStockKline = useStockAnalysisStore((s) => s.getStockKline);
  const startAnalysis = useStockAnalysisStore((s) => s.startAnalysis);
  const watchlistVersion = useStockAnalysisStore((s) => s.watchlistVersion);

  const [results, setResults] = useState<ScreenResult[]>([]);
  const [loading, setLoading] = useState(false);
  const [mode, setMode] = useState<"discover" | "screen">("discover");
  const [criteria, setCriteria] = useState<CriteriaState>(defaultCriteria);

  const discover = async () => {
    setLoading(true);
    try {
      const r = await invoke<ScreenResult[]>("discover_stock_candidates");
      if (Array.isArray(r)) { setResults(r); }
    } catch { /* 静默 */ }
    setLoading(false);
  };

  const screen = async () => {
    setLoading(true);
    try {
      const r = await invoke<ScreenResult[]>("screen_stocks", { criteria });
      if (Array.isArray(r)) { setResults(r); }
    } catch { /* 静默 */ }
    setLoading(false);
  };

  useEffect(() => {
    discover();
  }, [watchlistVersion]);

  const handleAnalyze = async (code: string) => {
    await getStockQuote(code);
    await getStockKline(code, "daily", 120);
    startAnalysis(code);
  };

  const nf = (v: number | null | undefined): number | undefined => v ?? undefined;

  return (
    <Card
      size="small"
      title="🔍 智能荐股"
      styles={{ body: { padding: "8px 10px" } }}
      extra={
        <div className="flex gap-1">
          <Button
            size="small"
            type={mode === "discover" ? "primary" : "default"}
            onClick={() => {
              setMode("discover");
              discover();
            }}
          >
            全市场
          </Button>
          <Button size="small" type={mode === "screen" ? "primary" : "default"} onClick={() => setMode("screen")}>
            自选筛选
          </Button>
        </div>
      }
    >
      {mode === "screen" && (
        <div className="flex flex-col gap-1 mb-2">
          {/* 基础条件 */}
          <div className="flex gap-1 flex-wrap items-center text-xs">
            <span className="text-gray-400">涨跌</span>
            <InputNumber
              size="small"
              style={{ width: 62 }}
              min={-10}
              max={10}
              step={0.5}
              value={criteria.minChangePct}
              onChange={(v) => setCriteria({ ...criteria, minChangePct: nf(v) })}
              placeholder="≥ %"
            />
            <span className="text-gray-400">~</span>
            <InputNumber
              size="small"
              style={{ width: 62 }}
              min={-10}
              max={10}
              step={0.5}
              value={criteria.maxChangePct}
              onChange={(v) => setCriteria({ ...criteria, maxChangePct: nf(v) })}
              placeholder="≤ %"
            />
            <span className="text-gray-400 ml-1">换手率≥</span>
            <InputNumber
              size="small"
              style={{ width: 60 }}
              min={0}
              max={50}
              step={0.5}
              value={criteria.turnoverRateMin}
              onChange={(v) => setCriteria({ ...criteria, turnoverRateMin: nf(v) })}
              placeholder="%"
            />
          </div>

          {/* 资金面 */}
          <div className="flex gap-1 flex-wrap items-center text-xs">
            <span className="text-gray-400">主力净流入≥</span>
            <InputNumber
              size="small"
              style={{ width: 72 }}
              min={0}
              step={100}
              value={criteria.mainInflowMin}
              onChange={(v) => setCriteria({ ...criteria, mainInflowMin: nf(v) })}
              placeholder="万元"
            />
            <span className="text-gray-400 ml-1">龙虎榜≥</span>
            <InputNumber
              size="small"
              style={{ width: 72 }}
              min={0}
              step={100}
              value={criteria.dragonTigerNetMin}
              onChange={(v) => setCriteria({ ...criteria, dragonTigerNetMin: nf(v) })}
              placeholder="万元"
            />
            <span className="text-gray-400 ml-1">北向持仓≥</span>
            <InputNumber
              size="small"
              style={{ width: 60 }}
              min={0}
              max={100}
              step={0.5}
              value={criteria.northboundRatioMin}
              onChange={(v) => setCriteria({ ...criteria, northboundRatioMin: nf(v) })}
              placeholder="%"
            />
          </div>

          {/* 技术指标 + 操作按钮 */}
          <div className="flex gap-1 flex-wrap items-center text-xs">
            <Checkbox
              checked={criteria.rsiOversold}
              onChange={(e) => setCriteria({ ...criteria, rsiOversold: e.target.checked })}
            >
              RSI超卖
            </Checkbox>
            <Checkbox
              checked={criteria.rsiOverbought}
              onChange={(e) => setCriteria({ ...criteria, rsiOverbought: e.target.checked })}
            >
              RSI超买
            </Checkbox>
            <Button size="small" icon={<SearchOutlined />} onClick={screen} loading={loading} type="primary">
              筛选
            </Button>
            <Button size="small" onClick={() => setCriteria(defaultCriteria)}>重置</Button>
          </div>
        </div>
      )}

      {loading
        ? <Spin size="small" />
        : results.length === 0
        ? (
          <Empty
            image={Empty.PRESENTED_IMAGE_SIMPLE}
            description={mode === "discover" ? "点击'全市场'发现候选" : "设置筛选条件后点击筛选"}
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
                actions={[<Tag key="score" color="blue" className="text-xs m-0">得分 {r.score}</Tag>]}
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
