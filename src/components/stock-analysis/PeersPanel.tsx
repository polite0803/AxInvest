import { invoke } from "@/lib/invoke";
import { Button, Card, Empty, Spin, Table } from "antd";
import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

interface PeerComparison {
  stockCode: string;
  stockName: string;
  pe: number | null;
  pb: number | null;
  roe: number | null;
  changePct: number;
  marketCap: number | null;
}

export function PeersPanel() {
  const { t } = useTranslation();
  const [peers, setPeers] = useState<PeerComparison[]>([]);
  const [loading, setLoading] = useState(false);
  const [fetchError, setFetchError] = useState(false);

  const load = useCallback(async () => {
    setLoading(true);
    setFetchError(false);
    try {
      const data = await invoke<PeerComparison[]>("get_stock_peers", { stockCode: "" }); // empty = auto
      setPeers(data ?? []);
    } catch {
      setFetchError(true);
    }
    setLoading(false);
  }, []);

  useEffect(() => {
    load();
  }, [load]);

  const columns = [
    { title: t("stockAnalysis.alert.code"), dataIndex: "stockCode", key: "code", width: 70 },
    { title: t("stockAnalysis.alert.name"), dataIndex: "stockName", key: "name", width: 80 },
    {
      title: "PE",
      dataIndex: "pe",
      key: "pe",
      width: 60,
      render: (v: number | null) => v != null ? v.toFixed(1) : "-",
    },
    {
      title: "PB",
      dataIndex: "pb",
      key: "pb",
      width: 60,
      render: (v: number | null) => v != null ? v.toFixed(1) : "-",
    },
    {
      title: "ROE",
      dataIndex: "roe",
      key: "roe",
      width: 60,
      render: (v: number | null) => v != null ? `${v.toFixed(1)}%` : "-",
    },
    {
      title: t("stockAnalysis.change"),
      dataIndex: "changePct",
      key: "change",
      width: 70,
      render: (v: number) => {
        const color = v >= 0 ? "var(--sa-red)" : "var(--sa-green)";
        return <span style={{ color, fontWeight: "bold" }}>{v >= 0 ? "+" : ""}{v.toFixed(2)}%</span>;
      },
    },
  ];

  return (
    <Card
      size="small"
      title={`🏢 ${t("stockAnalysis.peers")}`}
      styles={{ body: { padding: 0 } }}
      extra={
        <Button size="small" loading={loading} onClick={load}>{t("stockAnalysis.settings.panels.refresh")}</Button>
      }
    >
      {loading
        ? <Spin size="small" style={{ display: "block", margin: "16px auto" }} />
        : fetchError
        ? <Empty description={t("stockAnalysis.error")} image={Empty.PRESENTED_IMAGE_SIMPLE} />
        : <Table dataSource={peers} columns={columns} rowKey="stockCode" size="small" pagination={false} />}
    </Card>
  );
}
