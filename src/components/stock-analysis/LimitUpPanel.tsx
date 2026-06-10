import { List } from "@/components/common/AntdList";
import { invoke } from "@/lib/invoke";
import { useStockAnalysisStore } from "@/stores";
import { Button, Card, Spin, Tag } from "antd";
import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { PanelEmpty, type PanelEmptyKind } from "./PanelEmpty";
import { useStockAnalysisPage } from "./StockAnalysisPageContext";
import { checkVendorEnabled, PANEL_VENDORS } from "./vendorCheck";

interface LimitUpStock {
  code: string;
  name: string;
  price: number;
  changePct: number;
  turnoverRate: number;
  isSealed: boolean;
  boardCount: number;
}

interface LimitUpPanelProps {
  /** 是否显示外边框(ScreenerPage Collapse 内嵌时传 false) */
  bordered?: boolean;
}

export function LimitUpPanel({ bordered = true }: LimitUpPanelProps = {}) {
  const { t } = useTranslation();
  const { openDataSourceSettings } = useStockAnalysisPage();
  const getStockQuote = useStockAnalysisStore((s) => s.getStockQuote);
  const getStockKline = useStockAnalysisStore((s) => s.getStockKline);
  const startAnalysis = useStockAnalysisStore((s) => s.startAnalysis);
  const [stocks, setStocks] = useState<LimitUpStock[]>([]);
  const [loading, setLoading] = useState(false);
  const [emptyKind, setEmptyKind] = useState<PanelEmptyKind | null>(null);
  const [emptyVendors, setEmptyVendors] = useState<string[] | undefined>(undefined);

  const load = useCallback(async (silent = false) => {
    setLoading(true);
    setEmptyKind(null);
    setEmptyVendors(undefined);
    try {
      const check = await checkVendorEnabled("limitup", { silent });
      if (check.status === "disabled") {
        setStocks([]);
        setEmptyKind("vendorDisabled");
        setEmptyVendors(check.vendors);
        setLoading(false);
        return;
      }
      if (check.status === "backend_offline") {
        setStocks([]);
        setEmptyKind("backendOffline");
        setLoading(false);
        return;
      }
      const hot: any[] = await invoke("get_hot_stocks");
      if (!Array.isArray(hot)) { throw new Error("bad data"); }
      // 主排序：changePct 倒序；过滤：涨幅 >= 9.5% 的强势股
      // 数据源返回的 changePct 单位是 %（例如 9.8 / 10.0）
      const candidates = hot.filter((h) => (h.changePct ?? 0) >= 9.5);
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
            // 连板数估算：涨停板 9.5-19% 为 1 连板，19-29% 为 2 连板，以此类推
            boardCount: Math.max(1, Math.round((h.changePct ?? 0) / 10)),
          });
        } catch { /* 跳过单只 */ }
      }
      results.sort((a, b) => b.changePct - a.changePct);
      setStocks(results);
      if (results.length === 0) { setEmptyKind("noData"); }
    } catch {
      setStocks([]);
      setEmptyKind("connectionFailed");
    }
    setLoading(false);
  }, []);

  useEffect(() => {
    load(true); // 首次静默：避免每个面板都 toast 一次
  }, [load]);

  const analyze = async (code: string) => {
    await getStockQuote(code);
    await getStockKline(code, "daily", 120);
    startAnalysis(code);
  };

  return (
    <Card
      size="small"
      bordered={bordered}
      title={`🏆 ${t("stockAnalysis.settings.panels.limitUp")}`}
      styles={{ body: { padding: "4px 8px" } }}
      extra={
        <Button size="small" loading={loading} onClick={() => load()}>
          {t("stockAnalysis.settings.panels.refresh")}
        </Button>
      }
    >
      {loading
        ? <Spin size="small" style={{ display: "block", margin: "16px auto" }} />
        : emptyKind
        ? (
          <PanelEmpty
            kind={emptyKind}
            vendorNames={emptyVendors ?? PANEL_VENDORS.limitup}
            description={emptyKind === "noData" ? t("stockAnalysis.settings.panels.noLimitUp") : undefined}
            onOpenSettings={openDataSourceSettings}
          />
        )
        : (
          <List
            size="small"
            dataSource={stocks}
            renderItem={(s) => {
              const up = s.changePct >= 0;
              const changeColor = up ? "text-red-500" : "text-green-500";
              return (
                <List.Item
                  style={{ cursor: "pointer", padding: "3px 0" }}
                  onClick={() => analyze(s.code)}
                  actions={[
                    <Tag key="seal" color={s.isSealed ? "red" : "orange"} className="text-xs m-0">
                      {s.isSealed
                        ? t("stockAnalysis.settings.panels.sealed")
                        : t("stockAnalysis.settings.panels.opened")}
                    </Tag>,
                    s.boardCount > 1 && (
                      <Tag key="bc" color="volcano" className="text-xs m-0">
                        {t("stockAnalysis.settings.panels.consecutive", { n: s.boardCount })}
                      </Tag>
                    ),
                  ].filter(Boolean)}
                >
                  <div className="flex items-center gap-2 text-xs w-full">
                    <Tag className="m-0 text-xs">{s.code}</Tag>
                    <span className="flex-1 truncate">{s.name}</span>
                    <span className="font-mono">{s.price.toFixed(2)}</span>
                    <span className={changeColor}>
                      {up ? "+" : ""}
                      {s.changePct.toFixed(1)}%
                    </span>
                    <span className="text-gray-400">
                      {t("stockAnalysis.settings.panels.turnover")} {s.turnoverRate.toFixed(1)}%
                    </span>
                  </div>
                </List.Item>
              );
            }}
          />
        )}
    </Card>
  );
}
