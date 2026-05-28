import { invoke } from "@/lib/invoke";
import { Button, Card, Empty, Spin, Statistic } from "antd";
import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { checkVendorEnabled } from "./vendorCheck";

interface NbFlow {
  date: string;
  shFlow: number;
  szFlow: number;
  totalFlow: number;
}

export function NorthBoundPanel() {
  const { t } = useTranslation();
  const [flow, setFlow] = useState<NbFlow | null>(null);
  const [loading, setLoading] = useState(false);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const f: any = await invoke("get_north_bound_flow");
      if (f) {
        setFlow({
          date: f.date ?? "",
          shFlow: f.shFlow ?? f.sh_flow ?? 0,
          szFlow: f.szFlow ?? f.sz_flow ?? 0,
          totalFlow: f.totalFlow ?? f.total_flow ?? 0,
        });
      }
    } catch { /* */ }
    setLoading(false);
  }, []);

  useEffect(() => {
    load();
  }, [load]);

  const total = flow?.totalFlow ?? 0;
  const dir = total >= 0 ? t("stockAnalysis.settings.panels.inflow") : t("stockAnalysis.settings.panels.outflow");
  const color = total >= 0 ? "var(--sa-red)" : "var(--sa-green)";

  return (
    <Card
      size="small"
      title={`🧭 ${t("stockAnalysis.settings.panels.northBound")}`}
      styles={{ body: { padding: "4px 8px" } }}
      extra={
        <Button
          size="small"
          loading={loading}
          onClick={async () => {
            if (await checkVendorEnabled("north")) { load(); }
          }}
        >
          {t("stockAnalysis.settings.panels.refresh")}
        </Button>
      }
    >
      {loading
        ? <Spin size="small" />
        : !flow
        ? <Empty image={Empty.PRESENTED_IMAGE_SIMPLE} description={t("stockAnalysis.settings.panels.noNorthBound")} />
        : (
          <div className="text-center">
            <Statistic
              title={t("stockAnalysis.settings.panels.northTitle", { dir, date: flow.date })}
              value={Math.abs(total / 1e4).toFixed(1)}
              suffix={t("stockAnalysis.settings.panels.yiDisplay")}
              valueStyle={{ fontSize: 20, color, fontWeight: "bold" }}
            />
            <div className="grid grid-cols-2 gap-1 mt-1 text-xs text-gray-500">
              <span>
                {t("stockAnalysis.settings.panels.shFlow")}: {(flow.shFlow / 1e4).toFixed(1)}
                {t("stockAnalysis.settings.panels.yiDisplay")}
              </span>
              <span>
                {t("stockAnalysis.settings.panels.szFlow")}: {(flow.szFlow / 1e4).toFixed(1)}
                {t("stockAnalysis.settings.panels.yiDisplay")}
              </span>
            </div>
          </div>
        )}
    </Card>
  );
}
