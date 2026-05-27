import { invoke } from "@/lib/invoke";
import { Button, Card, Empty, Spin, Statistic } from "antd";
import { useCallback, useEffect, useState } from "react";

interface NbFlow {
  date: string;
  shFlow: number;
  szFlow: number;
  totalFlow: number;
}

export function NorthBoundPanel() {
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
  const dir = total >= 0 ? "流入" : "流出";
  const color = total >= 0 ? "var(--sa-red)" : "var(--sa-green)";

  return (
    <Card
      size="small"
      title="🧭 北向资金"
      styles={{ body: { padding: "4px 8px" } }}
      extra={<Button size="small" loading={loading} onClick={load}>刷新</Button>}
    >
      {loading
        ? <Spin size="small" />
        : !flow
        ? <Empty image={Empty.PRESENTED_IMAGE_SIMPLE} description="暂无北向数据" />
        : (
          <div className="text-center">
            <Statistic
              title={`北向资金 ${dir} (${flow.date})`}
              value={Math.abs(total / 1e4).toFixed(1)}
              suffix="亿"
              valueStyle={{ fontSize: 20, color, fontWeight: "bold" }}
            />
            <div className="grid grid-cols-2 gap-1 mt-1 text-xs text-gray-500">
              <span>沪股通: {(flow.shFlow / 1e4).toFixed(1)}亿</span>
              <span>深股通: {(flow.szFlow / 1e4).toFixed(1)}亿</span>
            </div>
          </div>
        )}
    </Card>
  );
}
