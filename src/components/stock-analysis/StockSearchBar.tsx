import { List } from "@/components/common/AntdList";
import { useStockAnalysisStore } from "@/stores";
import { Button, Input } from "antd";
import { useEffect } from "react";
import { useTranslation } from "react-i18next";

export function StockSearchBar() {
  const { t } = useTranslation();
  const searchKeyword = useStockAnalysisStore((s) => s.searchKeyword);
  const searchResults = useStockAnalysisStore((s) => s.searchResults);
  const searchStock = useStockAnalysisStore((s) => s.searchStock);
  const startAnalysis = useStockAnalysisStore((s) => s.startAnalysis);
  const getStockQuote = useStockAnalysisStore((s) => s.getStockQuote);
  const getStockKline = useStockAnalysisStore((s) => s.getStockKline);
  const status = useStockAnalysisStore((s) => s.status);
  const stockCode = useStockAnalysisStore((s) => s.stockCode);

  const isRunning = status === "loading" || status === "running";

  // Ctrl+K / Cmd+K 聚焦到搜索框
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && e.key === "k") {
        e.preventDefault();
        const el = document.querySelector<HTMLInputElement>(".stock-search-input input");
        el?.focus();
      }
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, []);

  return (
    <div className="flex flex-col gap-2">
      <div className="flex gap-2 items-center">
        <Input.Search
          className="stock-search-input"
          data-testid="stock-analysis-search-input"
          placeholder={`${t("stockAnalysis.searchPlaceholder")} (Ctrl+K)`}
          value={searchKeyword}
          onChange={(e) => searchStock(e.target.value)}
          onSearch={(value) => searchStock(value, true)}
          style={{ maxWidth: 300 }}
          loading={status === "loading"}
        />
        <Button
          type="primary"
          disabled={!stockCode || isRunning}
          loading={isRunning}
          onClick={() => {
            if (stockCode) {
              startAnalysis(stockCode);
            }
          }}
        >
          {isRunning ? t("stockAnalysis.analyzing") : t("stockAnalysis.startAnalysis")}
        </Button>
      </div>
      {searchResults.length > 0 && (
        <List
          size="small"
          bordered
          dataSource={searchResults}
          style={{ maxWidth: 300 }}
          renderItem={(item) => (
            <List.Item
              style={{ cursor: "pointer" }}
              onClick={() => {
                getStockQuote(item.code);
                getStockKline(item.code, "daily", 120);
                useStockAnalysisStore.setState({ searchResults: [] });
              }}
            >
              {item.code} — {item.name}
            </List.Item>
          )}
        />
      )}
    </div>
  );
}
