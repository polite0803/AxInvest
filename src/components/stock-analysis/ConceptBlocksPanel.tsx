import { invoke } from "@/lib/invoke";
import { Button, Card, Empty, Spin, Tag } from "antd";
import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

interface BlockItem {
  name: string;
  changePct: number | null;
}

interface ConceptBlocks {
  stockCode: string;
  industry: string;
  concepts: BlockItem[];
  regions: BlockItem[];
}

export function ConceptBlocksPanel({ stockCode }: { stockCode: string }) {
  const { t } = useTranslation();
  const [data, setData] = useState<ConceptBlocks | null>(null);
  const [loading, setLoading] = useState(false);
  const [fetchError, setFetchError] = useState(false);

  const load = useCallback(async () => {
    if (!stockCode) { return; }
    setLoading(true);
    setFetchError(false);
    try {
      const result = await invoke<ConceptBlocks | null>("get_stock_concept_blocks", { stockCode });
      setData(result);
    } catch {
      setFetchError(true);
    }
    setLoading(false);
  }, [stockCode]);

  useEffect(() => {
    load();
  }, [load]);

  const renderBlock = (items: BlockItem[], color: string) => (
    <div className="flex flex-wrap gap-1 mt-1">
      {items.map((b) => (
        <Tag key={b.name} color={color} style={{ fontSize: 11, margin: 0 }}>
          {b.name}
          {b.changePct != null ? ` ${b.changePct >= 0 ? "+" : ""}${b.changePct.toFixed(1)}%` : ""}
        </Tag>
      ))}
    </div>
  );

  return (
    <Card
      size="small"
      title={`🏷️ ${t("stockAnalysis.conceptBlocks")}`}
      styles={{ body: { padding: "8px 10px" } }}
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
        : !data
        ? <Empty description={t("stockAnalysis.noRecords")} image={Empty.PRESENTED_IMAGE_SIMPLE} />
        : (
          <div className="space-y-2">
            <div>
              <span className="text-xs font-medium">{t("stockAnalysis.industry")}:</span>
              <Tag color="blue" style={{ marginLeft: 4 }}>{data.industry}</Tag>
            </div>
            {data.concepts.length > 0 && (
              <div>
                <span className="text-xs font-medium">{t("stockAnalysis.concepts")}:</span>
                {renderBlock(data.concepts, "purple")}
              </div>
            )}
            {data.regions.length > 0 && (
              <div>
                <span className="text-xs font-medium">{t("stockAnalysis.regions")}:</span>
                {renderBlock(data.regions, "cyan")}
              </div>
            )}
          </div>
        )}
    </Card>
  );
}
