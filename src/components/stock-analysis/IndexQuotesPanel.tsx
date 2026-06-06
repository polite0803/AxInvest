import { invoke } from "@/lib/invoke";
import { Button, Card, Empty, Spin, Table } from "antd";
import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

interface IndexQuote {
  code: string;
  name: string;
  price: number;
  pre_close: number;
  change_pct: number;
  volume: number;
  amount: number;
}

export function IndexQuotesPanel() {
  const { t } = useTranslation();
  const [quotes, setQuotes] = useState<IndexQuote[]>([]);
  const [loading, setLoading] = useState(false);
  const [fetchError, setFetchError] = useState(false);

  const load = useCallback(async () => {
    setLoading(true);
    setFetchError(false);
    try {
      const data = await invoke<IndexQuote[]>("get_index_quotes");
      setQuotes(data ?? []);
    } catch {
      setFetchError(true);
    }
    setLoading(false);
  }, []);

  useEffect(() => {
    load();
  }, [load]);

  const columns = [
    { title: t("stockAnalysis.alert.name"), dataIndex: "name", key: "name", width: 100 },
    {
      title: t("stockAnalysis.price"),
      dataIndex: "price",
      key: "price",
      width: 80,
      render: (v: number) => v.toFixed(2),
    },
    {
      title: t("stockAnalysis.change"),
      dataIndex: "change_pct",
      key: "change_pct",
      width: 80,
      render: (v: number) => {
        const color = v >= 0 ? "var(--sa-red)" : "var(--sa-green)";
        return <span style={{ color, fontWeight: "bold" }}>{v >= 0 ? "+" : ""}{v.toFixed(2)}%</span>;
      },
    },
  ];

  return (
    <Card
      size="small"
      title={`📈 ${t("stockAnalysis.indexQuotes")}`}
      styles={{ body: { padding: 0 } }}
      extra={
        <Button size="small" loading={loading} onClick={load}>{t("stockAnalysis.settings.panels.refresh")}</Button>
      }
    >
      {loading
        ? <Spin size="small" style={{ display: "block", margin: "16px auto" }} />
        : fetchError
        ? <Empty description={t("stockAnalysis.error")} image={Empty.PRESENTED_IMAGE_SIMPLE} />
        : <Table dataSource={quotes} columns={columns} rowKey="code" size="small" pagination={false} />}
    </Card>
  );
}
