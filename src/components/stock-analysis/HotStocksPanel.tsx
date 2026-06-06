import { invoke } from "@/lib/invoke";
import { Button, Card, Empty, Spin, Table, Tag } from "antd";
import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

interface HotStock {
  stock_code: string;
  stock_name: string;
  change_pct: f64;
  turnover_rate: number | null;
  reason_tags: string[];
  sector: string | null;
}

type f64 = number;

export function HotStocksPanel() {
  const { t } = useTranslation();
  const [stocks, setStocks] = useState<HotStock[]>([]);
  const [loading, setLoading] = useState(false);
  const [fetchError, setFetchError] = useState(false);

  const load = useCallback(async () => {
    setLoading(true);
    setFetchError(false);
    try {
      const data = await invoke<HotStock[]>("get_hot_stocks");
      setStocks(data ?? []);
    } catch {
      setFetchError(true);
    }
    setLoading(false);
  }, []);

  useEffect(() => {
    load();
  }, [load]);

  const columns = [
    { title: t("stockAnalysis.alert.code"), dataIndex: "stock_code", key: "code", width: 70 },
    { title: t("stockAnalysis.alert.name"), dataIndex: "stock_name", key: "name", width: 80 },
    {
      title: t("stockAnalysis.change"),
      dataIndex: "change_pct",
      key: "change",
      width: 70,
      render: (v: number) => {
        const color = v >= 0 ? "var(--sa-red)" : "var(--sa-green)";
        return <span style={{ color, fontWeight: "bold" }}>{v >= 0 ? "+" : ""}{v.toFixed(2)}%</span>;
      },
    },
    {
      title: t("stockAnalysis.reason"),
      dataIndex: "reason_tags",
      key: "reason",
      render: (tags: string[]) => tags.map((t) => <Tag key={t} style={{ fontSize: 10 }}>{t}</Tag>),
    },
    { title: t("stockAnalysis.industry"), dataIndex: "sector", key: "sector", width: 80 },
  ];

  return (
    <Card
      size="small"
      title={`🔥 ${t("stockAnalysis.hotStocks")}`}
      styles={{ body: { padding: 0 } }}
      extra={
        <Button size="small" loading={loading} onClick={load}>{t("stockAnalysis.settings.panels.refresh")}</Button>
      }
    >
      {loading
        ? <Spin size="small" style={{ display: "block", margin: "16px auto" }} />
        : fetchError
        ? <Empty description={t("stockAnalysis.error")} image={Empty.PRESENTED_IMAGE_SIMPLE} />
        : (
          <Table
            dataSource={stocks}
            columns={columns}
            rowKey="stock_code"
            size="small"
            pagination={{ pageSize: 10 }}
          />
        )}
    </Card>
  );
}
