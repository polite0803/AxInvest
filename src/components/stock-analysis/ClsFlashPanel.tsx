import { invoke } from "@/lib/invoke";
import { Button, Card, Empty, Spin, Tag, Timeline } from "antd";
import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

interface ClsFlashItem {
  title: string;
  content: string;
  publishTime: string;
  source: string | null;
}

export function ClsFlashPanel() {
  const { t } = useTranslation();
  const [items, setItems] = useState<ClsFlashItem[]>([]);
  const [loading, setLoading] = useState(false);
  const [fetchError, setFetchError] = useState(false);

  const load = useCallback(async () => {
    setLoading(true);
    setFetchError(false);
    try {
      const data = await invoke<ClsFlashItem[]>("get_cls_flash");
      setItems(data ?? []);
    } catch {
      setFetchError(true);
    }
    setLoading(false);
  }, []);

  useEffect(() => {
    load();
  }, [load]);

  return (
    <Card
      size="small"
      title={`⚡ ${t("stockAnalysis.clsFlash")}`}
      styles={{ body: { padding: "8px 10px", maxHeight: 400, overflowY: "auto" } }}
      extra={
        <Button size="small" loading={loading} onClick={load}>{t("stockAnalysis.settings.panels.refresh")}</Button>
      }
    >
      {loading
        ? <Spin size="small" style={{ display: "block", margin: "16px auto" }} />
        : fetchError
        ? <Empty description={t("stockAnalysis.error")} image={Empty.PRESENTED_IMAGE_SIMPLE} />
        : items.length === 0
        ? <Empty description={t("stockAnalysis.noRecords")} image={Empty.PRESENTED_IMAGE_SIMPLE} />
        : (
          <Timeline
            items={items.slice(0, 20).map((item) => ({
              children: (
                <div>
                  <div className="text-xs font-medium">{item.title}</div>
                  <div className="text-xs text-gray-500 mt-1">{item.content}</div>
                  <div className="text-xs text-gray-400 mt-1">
                    {item.publishTime}
                    {item.source && <Tag style={{ marginLeft: 4, fontSize: 10 }}>{item.source}</Tag>}
                  </div>
                </div>
              ),
            }))}
          />
        )}
    </Card>
  );
}
