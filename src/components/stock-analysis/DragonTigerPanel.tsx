import { invoke } from "@/lib/invoke";
import { useStockAnalysisStore } from "@/stores";
import { Button, Card, Empty, List, Spin, Tag } from "antd";
import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

interface DtEntry {
  code: string;
  name: string;
  date: string;
  netBuy: number;
  buyAmount: number;
  sellAmount: number;
  reason: string;
}

export function DragonTigerPanel() {
  const { t } = useTranslation();
  const getStockQuote = useStockAnalysisStore((s) => s.getStockQuote);
  const getStockKline = useStockAnalysisStore((s) => s.getStockKline);
  const startAnalysis = useStockAnalysisStore((s) => s.startAnalysis);
  const [entries, setEntries] = useState<DtEntry[]>([]);
  const [loading, setLoading] = useState(false);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const list: any[] = await invoke("get_market_dragon_tiger");
      if (Array.isArray(list)) {
        setEntries(
          list.slice(0, 20).map((e: any) => ({
            code: e.stockCode ?? e.stock_code ?? "",
            name: e.stockName ?? e.stock_name ?? "",
            date: e.date ?? "",
            netBuy: e.netBuy ?? e.net_buy ?? 0,
            buyAmount: e.buyAmount ?? e.buy_amount ?? 0,
            sellAmount: e.sellAmount ?? e.sell_amount ?? 0,
            reason: e.reason ?? "",
          })),
        );
      }
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
      title={`🐉 ${t("stockAnalysis.settings.panels.dragonTiger")}`}
      styles={{ body: { padding: "4px 8px" } }}
      extra={
        <Button size="small" loading={loading} onClick={load}>{t("stockAnalysis.settings.panels.refresh")}</Button>
      }
    >
      {loading
        ? <Spin size="small" />
        : entries.length === 0
        ? <Empty image={Empty.PRESENTED_IMAGE_SIMPLE} description={t("stockAnalysis.settings.panels.noDragonTiger")} />
        : (
          <List
            size="small"
            dataSource={entries}
            renderItem={(e) => (
              <List.Item
                style={{ cursor: "pointer", padding: "3px 0" }}
                onClick={() => analyze(e.code)}
                actions={[
                  <Tag key="net" color={e.netBuy > 0 ? "red" : "green"} className="text-xs m-0">
                    {e.netBuy > 0
                      ? t("stockAnalysis.settings.panels.netBuy")
                      : t("stockAnalysis.settings.panels.netSell")} {(Math.abs(e.netBuy) / 1e4).toFixed(0)}
                    {t("stockAnalysis.wanUnit")}
                  </Tag>,
                ]}
              >
                <div className="flex items-center gap-2 text-xs w-full">
                  <Tag className="m-0 text-xs">{e.code}</Tag>
                  <span className="flex-1 truncate">{e.name}</span>
                  {e.reason && <Tag color="orange" className="text-xs m-0">{e.reason}</Tag>}
                </div>
              </List.Item>
            )}
          />
        )}
    </Card>
  );
}
