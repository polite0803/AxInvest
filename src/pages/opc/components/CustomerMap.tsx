// SPDX-License-Identifier: AGPL-3.0-only

// CustomerMap — 客户地理分布地图（ECharts geo 散点打点，中国 / 世界可切换）

import * as echarts from "echarts";
import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

import { Empty, Segmented } from "antd";

import type { Customer } from "../utils/constants";

type Scope = "china" | "world";

const GEO_URLS: Record<Scope, string> = {
  china: "/customer-map/china.json",
  world: "/customer-map/world.json",
};

interface CustomerMapProps {
  customers: Customer[];
  height?: number;
}

export function CustomerMap({ customers, height = 420 }: CustomerMapProps) {
  const { t } = useTranslation();
  const [scope, setScope] = useState<Scope>("china");
  const chartRef = useRef<HTMLDivElement>(null);
  const instRef = useRef<echarts.ECharts | null>(null);

  // 有经纬度的客户 → 地图打点
  const points = useMemo(
    () =>
      customers
        .filter((c) => c.latitude != null && c.longitude != null)
        .map((c) => ({
          name: c.name,
          type: c.customer_type,
          value: [c.longitude as number, c.latitude as number, c.total_revenue],
        })),
    [customers],
  );

  useEffect(() => {
    if (!chartRef.current) { return; }
    instRef.current = echarts.init(chartRef.current, undefined, { renderer: "canvas" });
    return () => {
      instRef.current?.dispose();
      instRef.current = null;
    };
  }, []);

  useEffect(() => {
    if (!instRef.current) { return; }
    const inst = instRef.current;
    // 无经纬度客户时本地守卫：不请求底图、不 setOption
    if (points.length === 0) {
      instRef.current?.clear();
      return;
    }

    let cancelled = false;

    fetch(GEO_URLS[scope])
      .then((r) => r.json())
      .then((geoJson) => {
        if (cancelled) { return; }
        echarts.registerMap(scope, geoJson as Parameters<typeof echarts.registerMap>[1]);
        inst.clear();
        inst.setOption(
          {
            tooltip: {
              trigger: "item",
              formatter: (params: unknown) => {
                const p2 = params as { name?: string; data?: { name?: string; value?: number[] } };
                const d = p2.data;
                if (!d || !Array.isArray(d.value) || d.value.length < 3) {
                  return p2.name ?? "";
                }
                return `${d.name ?? ""}<br/>¥${Number(d.value[2]).toFixed(2)}`;
              },
            },
            geo: {
              map: scope,
              roam: true,
              itemStyle: {
                areaColor: "#eef2f7",
                borderColor: "#b8c4d2",
              },
              emphasis: { label: { show: false }, itemStyle: { areaColor: "#dbe6f1" } },
            },
            series: [
              {
                name: t("opc.customerMap.title"),
                type: "scatter",
                coordinateSystem: "geo",
                symbolSize: 9,
                itemStyle: {
                  // 按客户类型着色：business 紫 / consumer 青
                  color: (params: unknown) => {
                    const p2 = params as { data: { type?: string } };
                    return p2.data?.type === "business" ? "#722ed1" : "#13c2c2";
                  },
                },
                label: {
                  show: true,
                  formatter: "{b}",
                  position: "right",
                  fontSize: 10,
                  color: "#333",
                },
                data: points.map((p) => ({ name: p.name, type: p.type, value: p.value })),
              },
            ],
          },
          true,
        );
      })
      .catch((e) => {
        console.error("[CustomerMap] 底图加载失败:", e);
      });

    return () => {
      cancelled = true;
    };
  }, [scope, points, t]);

  useEffect(() => {
    const onResize = () => instRef.current?.resize();
    window.addEventListener("resize", onResize);
    return () => window.removeEventListener("resize", onResize);
  }, []);

  return (
    <div>
      <Segmented
        options={[
          { label: t("opc.customerMap.china"), value: "china" },
          { label: t("opc.customerMap.world"), value: "world" },
        ]}
        value={scope}
        onChange={(v) => setScope(v as Scope)}
        style={{ marginBottom: 8 }}
      />
      {points.length === 0
        ? <Empty description={t("opc.customerMap.empty")} style={{ padding: 32 }} />
        : <div ref={chartRef} style={{ width: "100%", height }} />}
    </div>
  );
}
