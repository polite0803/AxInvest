import { useProviderStore, useStockAnalysisStore } from "@/stores";
import { Button, DatePicker, Input, List } from "antd";
import dayjs from "dayjs";
import { useState } from "react";
import { useTranslation } from "react-i18next";

export function StockSearchBar() {
  const { t } = useTranslation();
  const searchKeyword = useStockAnalysisStore((s) => s.searchKeyword);
  const searchResults = useStockAnalysisStore((s) => s.searchResults);
  const searchStock = useStockAnalysisStore((s) => s.searchStock);
  const startAnalysis = useStockAnalysisStore((s) => s.startAnalysis);
  const status = useStockAnalysisStore((s) => s.status);
  const stockCode = useStockAnalysisStore((s) => s.stockCode);

  const isRunning = status === "loading" || status === "running";
  const [analysisDate, setAnalysisDate] = useState(dayjs());
  const defaultProviderId = useProviderStore(
    (s) => s.providers.find((p) => p.enabled)?.id ?? "",
  );

  return (
    <div className="flex flex-col gap-2">
      <div className="flex gap-2 items-center">
        <Input.Search
          placeholder={t("stockAnalysis.searchPlaceholder")}
          value={searchKeyword}
          onChange={(e) => searchStock(e.target.value)}
          onSearch={(value) => searchStock(value)}
          style={{ maxWidth: 300 }}
          loading={status === "loading"}
        />
        <DatePicker
          value={analysisDate}
          onChange={(d) => setAnalysisDate(d || dayjs())}
          disabled={isRunning}
        />
        <Button
          type="primary"
          disabled={!stockCode || isRunning}
          loading={isRunning}
          onClick={() => {
            if (stockCode) {
              startAnalysis(stockCode, analysisDate.format("YYYY-MM-DD"), defaultProviderId);
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
                useStockAnalysisStore.getState().getStockQuote(item.code);
                useStockAnalysisStore.getState().getStockKline(item.code, "daily", 120);
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
