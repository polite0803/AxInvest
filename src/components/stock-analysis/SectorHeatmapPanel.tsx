import { invoke } from "@/lib/invoke";
import { useStockAnalysisStore } from "@/stores";
import { Button, Card, Empty, List, Spin, Tag } from "antd";
import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { checkVendorEnabled } from "./vendorCheck";

interface SectorEntry {
  name: string;
  changePct: number;
  turnover: number;
  leaderCode?: string;
  leaderName?: string;
  leaderChange?: number;
}

export function SectorHeatmapPanel() {
  const { t } = useTranslation();
  const getStockQuote = useStockAnalysisStore((s) => s.getStockQuote);
  const getStockKline = useStockAnalysisStore((s) => s.getStockKline);
  const startAnalysis = useStockAnalysisStore((s) => s.startAnalysis);
  const [sectors, setSectors] = useState<SectorEntry[]>([]);
  const [loading, setLoading] = useState(false);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const list: any[] = await invoke("get_industry_ranking");
      if (Array.isArray(list)) {
        setSectors(
          list.slice(0, 25).map((e: any) => ({
            name: e.industryName ?? e.industry_name ?? "",
            changePct: e.changePct ?? e.change_pct ?? 0,
            turnover: e.turnover ?? 0,
            leaderCode: e.leaderCode ?? e.leader_code,
            leaderName: e.leaderName ?? e.leader_name,
            leaderChange: e.leaderChangePct ?? e.leader_change_pct,
          })),
        );
      }
    } catch { /* */ }
    setLoading(false);
  }, []);

  useEffect(() => {
    load();
  }, [load]);

  const analyze = async (code: string) => {
    await getStockQuote(code);
    await getStockKline(code, "daily", 120);
    startAnalysis(code);
  };

  const getColor = (pct: number) => {
    if (pct >= 3) { return "red"; }
    if (pct >= 1) { return "orange"; }
    if (pct >= 0) { return "gold"; }
    if (pct >= -1) { return "green"; }
    if (pct >= -3) { return "cyan"; }
    return "blue";
  };

  return (
    <Card
      size="small"
      title={`🔥 ${t("stockAnalysis.settings.panels.sectorHeatmap")}`}
      styles={{ body: { padding: "4px 8px" } }}
      extra={
        <Button
          size="small"
          loading={loading}
          onClick={async () => {
            if (await checkVendorEnabled("sectors")) { load(); }
          }}
        >
          {t("stockAnalysis.settings.panels.refresh")}
        </Button>
      }
    >
      {loading
        ? <Spin size="small" />
        : sectors.length === 0
        ? <Empty image={Empty.PRESENTED_IMAGE_SIMPLE} description={t("stockAnalysis.settings.panels.noSector")} />
        : (
          <List
            size="small"
            dataSource={sectors}
            renderItem={(s) => (
              <List.Item
                style={{ cursor: "pointer", padding: "3px 0" }}
                onClick={() => {
                  if (s.leaderCode) { analyze(s.leaderCode); }
                }}
              >
                <div className="flex items-center gap-2 text-xs w-full">
                  <Tag color={getColor(s.changePct)} className="text-xs m-0 min-w-0 truncate">{s.name}</Tag>
                  <span className={s.changePct >= 0 ? "text-red-500" : "text-green-500"} style={{ minWidth: 52 }}>
                    {s.changePct >= 0 ? "+" : ""}
                    {s.changePct.toFixed(2)}%
                  </span>
                  {s.leaderName && (
                    <span className="text-gray-400 truncate">
                      🏷 {s.leaderName}
                    </span>
                  )}
                </div>
              </List.Item>
            )}
          />
        )}
    </Card>
  );
}
