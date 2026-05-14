import { useStockAnalysisStore } from "@/stores";
import { Card, Statistic, Tag } from "antd";
import { useTranslation } from "react-i18next";

export function StockQuoteCard() {
  const { t } = useTranslation();
  const quote = useStockAnalysisStore((s) => s.quote);
  const stockName = useStockAnalysisStore((s) => s.stockName);

  if (!quote) { return null; }

  const isUp = quote.changePct >= 0;
  const color = isUp ? "#cf1322" : "#3f8600";

  return (
    <Card size="small" title={`${quote.name || stockName} (${quote.code})`}>
      <div className="flex gap-4 items-center flex-wrap">
        <Statistic
          title={t("stockAnalysis.price")}
          value={quote.price}
          precision={2}
          valueStyle={{ color }}
        />
        <Tag color={isUp ? "red" : "green"}>
          {isUp ? "+" : ""}
          {quote.changePct.toFixed(2)}%
        </Tag>
        <div className="text-xs" style={{ color: "var(--color-text-secondary)" }}>
          {t("stockAnalysis.open")}: {quote.open} &nbsp;
          {t("stockAnalysis.high")}: {quote.high} &nbsp;
          {t("stockAnalysis.low")}: {quote.low} &nbsp;
          {t("stockAnalysis.volume")}: {(quote.volume / 10000).toFixed(1)}
          {t("stockAnalysis.volumeUnit")}
        </div>
      </div>
    </Card>
  );
}
