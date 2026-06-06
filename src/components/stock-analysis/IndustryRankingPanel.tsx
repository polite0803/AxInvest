import { invoke } from "@/lib/invoke";
import { Button, Card, Empty, Spin, Table } from "antd";
import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

interface IndustryRank {
  industryName: string;
  changePct: number;
  turnover: number | null;
  leaderCode: string | null;
  leaderName: string | null;
  leaderChangePct: number | null;
}

export function IndustryRankingPanel() {
  const { t } = useTranslation();
  const [rankings, setRankings] = useState<IndustryRank[]>([]);
  const [loading, setLoading] = useState(false);
  const [fetchError, setFetchError] = useState(false);

  const load = useCallback(async () => {
    setLoading(true);
    setFetchError(false);
    try {
      const data = await invoke<IndustryRank[]>("get_industry_ranking");
      setRankings(data ?? []);
    } catch {
      setFetchError(true);
    }
    setLoading(false);
  }, []);

  useEffect(() => {
    load();
  }, [load]);

  const columns = [
    { title: t("stockAnalysis.industry"), dataIndex: "industryName", key: "name" },
    {
      title: t("stockAnalysis.change"),
      dataIndex: "changePct",
      key: "change",
      width: 80,
      render: (v: number) => {
        const color = v >= 0 ? "var(--sa-red)" : "var(--sa-green)";
        return <span style={{ color, fontWeight: "bold" }}>{v >= 0 ? "+" : ""}{v.toFixed(2)}%</span>;
      },
    },
    {
      title: t("stockAnalysis.turnoverRate"),
      dataIndex: "turnover",
      key: "turnover",
      width: 70,
      render: (v: number | null) => v != null ? `${v.toFixed(1)}%` : "-",
    },
    {
      title: t("stockAnalysis.leader"),
      dataIndex: "leaderName",
      key: "leader",
      width: 80,
      render: (v: string | null, r: IndustryRank) =>
        v
          ? (
            <span>
              {v}{" "}
              {r.leaderChangePct != null ? `${r.leaderChangePct >= 0 ? "+" : ""}${r.leaderChangePct.toFixed(2)}%` : ""}
            </span>
          )
          : "-",
    },
  ];

  return (
    <Card
      size="small"
      title={`📊 ${t("stockAnalysis.industryRanking")}`}
      styles={{ body: { padding: 0 } }}
      extra={
        <Button size="small" loading={loading} onClick={load}>{t("stockAnalysis.settings.panels.refresh")}</Button>
      }
    >
      {loading
        ? <Spin size="small" style={{ display: "block", margin: "16px auto" }} />
        : fetchError
        ? <Empty description={t("stockAnalysis.error")} image={Empty.PRESENTED_IMAGE_SIMPLE} />
        : <Table dataSource={rankings} columns={columns} rowKey="industryName" size="small" pagination={false} />}
    </Card>
  );
}
