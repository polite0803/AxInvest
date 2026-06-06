import { invoke } from "@/lib/invoke";
import { Button, Card, Empty, Spin, Table, Tag } from "antd";
import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

interface Announcement {
  title: string;
  stockCode: string;
  stockName: string | null;
  announceDate: string;
  annType: string | null;
  pdfUrl: string | null;
}

export function AnnouncementsPanel({ stockCode }: { stockCode: string }) {
  const { t } = useTranslation();
  const [items, setItems] = useState<Announcement[]>([]);
  const [loading, setLoading] = useState(false);
  const [fetchError, setFetchError] = useState(false);

  const load = useCallback(async () => {
    if (!stockCode) { return; }
    setLoading(true);
    setFetchError(false);
    try {
      const data = await invoke<Announcement[]>("get_stock_announcements", { stockCode });
      setItems(data ?? []);
    } catch {
      setFetchError(true);
    }
    setLoading(false);
  }, [stockCode]);

  useEffect(() => {
    load();
  }, [load]);

  const columns = [
    {
      title: t("stockAnalysis.alert.name"),
      dataIndex: "title",
      key: "title",
      ellipsis: true,
      render: (v: string, r: Announcement) =>
        r.pdfUrl ? <a href={r.pdfUrl} target="_blank" rel="noreferrer">{v}</a> : v,
    },
    { title: t("stockAnalysis.alert.date"), dataIndex: "announceDate", key: "date", width: 100 },
    {
      title: t("stockAnalysis.type"),
      dataIndex: "annType",
      key: "type",
      width: 80,
      render: (v: string | null) => v ? <Tag>{v}</Tag> : "-",
    },
  ];

  return (
    <Card
      size="small"
      title={`📋 ${t("stockAnalysis.announcements")}`}
      styles={{ body: { padding: 0 } }}
      extra={
        <Button size="small" loading={loading} onClick={load}>{t("stockAnalysis.settings.panels.refresh")}</Button>
      }
    >
      {!stockCode
        ? <Empty description={t("stockAnalysis.searchPlaceholder")} image={Empty.PRESENTED_IMAGE_SIMPLE} />
        : loading
        ? <Spin size="small" style={{ display: "block", margin: "16px auto" }} />
        : fetchError
        ? <Empty description={t("stockAnalysis.error")} image={Empty.PRESENTED_IMAGE_SIMPLE} />
        : (
          <Table
            dataSource={items}
            columns={columns}
            rowKey={(r) => r.title + r.announceDate}
            size="small"
            pagination={{ pageSize: 5 }}
          />
        )}
    </Card>
  );
}
