import { invoke } from "@/lib/invoke";
import { useStockAnalysisStore } from "@/stores";
import { Button, Card, Empty, List, Spin, Tag } from "antd";
import { useCallback, useEffect, useState } from "react";

interface LimitUpStock {
  code: string;
  name: string;
  price: number;
  changePct: number;
  turnoverRate: number;
  isSealed: boolean;
  boardCount: number;
}

export function LimitUpPanel() {
  const getStockQuote = useStockAnalysisStore((s) => s.getStockQuote);
  const getStockKline = useStockAnalysisStore((s) => s.getStockKline);
  const startAnalysis = useStockAnalysisStore((s) => s.startAnalysis);
  const [stocks, setStocks] = useState<LimitUpStock[]>([]);
  const [loading, setLoading] = useState(false);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const hot: any[] = await invoke("get_hot_stocks");
      if (!Array.isArray(hot)) { return; }
      const candidates = hot.filter((h) => Math.abs(h.changePct ?? 0) >= 9.5);
      const results: LimitUpStock[] = [];
      for (const h of candidates.slice(0, 30)) {
        try {
          const q = await invoke<any>("get_stock_quote", { stockCode: h.stockCode ?? h.stock_code });
          const price = q?.price ?? 0;
          const limitUp = q?.limitUp ?? q?.limit_up ?? 0;
          results.push({
            code: h.stockCode ?? h.stock_code,
            name: h.stockName ?? h.stock_name ?? "",
            price,
            changePct: h.changePct ?? 0,
            turnoverRate: q?.turnoverRate ?? q?.turnover_rate ?? 0,
            isSealed: limitUp > 0 && Math.abs(price - limitUp) < 0.01,
            boardCount: Math.round((h.changePct ?? 0) / 10),
          });
        } catch { /* skip */ }
      }
      results.sort((a, b) => b.changePct - a.changePct);
      setStocks(results);
    } catch { /* */ }
    setLoading(false);
  }, []);

  useEffect(() => {
    load();
  }, [load]);

  const analyze = async (code: string) => {
    await getStockQuote(code);
    await getStockKline(code, "daily", 120);
    startAnalysis(code);
  };

  return (
    <Card
      size="small"
      title="🏆 涨停板"
      styles={{ body: { padding: "4px 8px" } }}
      extra={<Button size="small" loading={loading} onClick={load}>刷新</Button>}
    >
      {loading
        ? <Spin size="small" />
        : stocks.length === 0
        ? <Empty image={Empty.PRESENTED_IMAGE_SIMPLE} description="暂无涨停数据" />
        : (
          <List
            size="small"
            dataSource={stocks}
            renderItem={(s) => (
              <List.Item
                style={{ cursor: "pointer", padding: "3px 0" }}
                onClick={() => analyze(s.code)}
                actions={[
                  <Tag key="seal" color={s.isSealed ? "red" : "orange"} className="text-xs m-0">
                    {s.isSealed ? "封板" : "开板"}
                  </Tag>,
                  s.boardCount > 1 && <Tag key="bc" color="volcano" className="text-xs m-0">{s.boardCount}连板</Tag>,
                ].filter(Boolean)}
              >
                <div className="flex items-center gap-2 text-xs w-full">
                  <Tag className="m-0 text-xs">{s.code}</Tag>
                  <span className="flex-1 truncate">{s.name}</span>
                  <span className="font-mono">{s.price.toFixed(2)}</span>
                  <span className="text-red-500">+{s.changePct.toFixed(1)}%</span>
                  <span className="text-gray-400">换手 {s.turnoverRate.toFixed(1)}%</span>
                </div>
              </List.Item>
            )}
          />
        )}
    </Card>
  );
}
