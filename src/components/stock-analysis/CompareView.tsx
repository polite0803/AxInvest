import { invoke } from "@/lib/invoke";
import type { StockQuote } from "@/types";
import { SwapOutlined } from "@ant-design/icons";
import { Button, Card, Input } from "antd";
import { useState } from "react";
import { useTranslation } from "react-i18next";

export function CompareView() {
  const { t } = useTranslation();
  const [code1, setCode1] = useState("");
  const [code2, setCode2] = useState("");
  const [quote1, setQuote1] = useState<StockQuote | null>(null);
  const [quote2, setQuote2] = useState<StockQuote | null>(null);
  const [loading, setLoading] = useState(false);

  const compare = async () => {
    if (!code1 || !code2) { return; }
    setLoading(true);
    try {
      const [q1, q2] = await Promise.all([
        invoke<StockQuote>("get_stock_quote", { stockCode: code1 }),
        invoke<StockQuote>("get_stock_quote", { stockCode: code2 }),
      ]);
      setQuote1(q1);
      setQuote2(q2);
    } catch {
      // 静默处理
    } finally {
      setLoading(false);
    }
  };

  return (
    <Card
      size="small"
      title={
        <span>
          <SwapOutlined /> {t("stockAnalysis.compare")}
        </span>
      }
    >
      <div className="flex gap-2 mb-2">
        <Input
          placeholder={t("stockAnalysis.stockCode1")}
          value={code1}
          onChange={(e) => setCode1(e.target.value)}
          style={{ width: 120 }}
          size="small"
        />
        <Input
          placeholder={t("stockAnalysis.stockCode2")}
          value={code2}
          onChange={(e) => setCode2(e.target.value)}
          style={{ width: 120 }}
          size="small"
        />
        <Button size="small" onClick={compare} disabled={!code1 || !code2} loading={loading}>
          {t("stockAnalysis.compareBtn")}
        </Button>
      </div>
      {quote1 && quote2 && (
        <table className="text-xs w-full">
          <thead>
            <tr>
              <th></th>
              <th>
                {quote1.name}
                <br />
                {quote1.code}
              </th>
              <th>
                {quote2.name}
                <br />
                {quote2.code}
              </th>
            </tr>
          </thead>
          <tbody>
            <tr>
              <td>{t("stockAnalysis.price")}</td>
              <td className="text-right">{quote1.price}</td>
              <td className="text-right">{quote2.price}</td>
            </tr>
            <tr>
              <td>{t("stockAnalysis.change")}</td>
              <td className="text-right" style={{ color: quote1.changePct >= 0 ? "#cf1322" : "#3f8600" }}>
                {quote1.changePct.toFixed(2)}%
              </td>
              <td className="text-right" style={{ color: quote2.changePct >= 0 ? "#cf1322" : "#3f8600" }}>
                {quote2.changePct.toFixed(2)}%
              </td>
            </tr>
            <tr>
              <td>PE</td>
              <td className="text-right">{quote1.pe ?? "-"}</td>
              <td className="text-right">{quote2.pe ?? "-"}</td>
            </tr>
            <tr>
              <td>PB</td>
              <td className="text-right">{quote1.pb ?? "-"}</td>
              <td className="text-right">{quote2.pb ?? "-"}</td>
            </tr>
            <tr>
              <td>{t("stockAnalysis.volume")}</td>
              <td className="text-right">{(quote1.volume / 10000).toFixed(1)}{t("stockAnalysis.volumeUnit")}</td>
              <td className="text-right">{(quote2.volume / 10000).toFixed(1)}{t("stockAnalysis.volumeUnit")}</td>
            </tr>
          </tbody>
        </table>
      )}
    </Card>
  );
}
