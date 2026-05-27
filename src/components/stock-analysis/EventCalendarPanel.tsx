import { invoke } from "@/lib/invoke";
import { useStockAnalysisStore } from "@/stores";
import { Button, Card, Empty, List, Spin, Tag } from "antd";
import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

interface EventItem {
  type: string;
  code: string;
  name: string;
  date: string;
  detail: string;
}

export function EventCalendarPanel() {
  const { t } = useTranslation();
  const getStockQuote = useStockAnalysisStore((s) => s.getStockQuote);
  const getStockKline = useStockAnalysisStore((s) => s.getStockKline);
  const startAnalysis = useStockAnalysisStore((s) => s.startAnalysis);
  const [events, setEvents] = useState<EventItem[]>([]);
  const [loading, setLoading] = useState(false);

  const load = useCallback(async () => {
    setLoading(true);
    const items: EventItem[] = [];
    try {
      // 从自选股批量获取解禁数据
      try {
        const wl: any[] = await invoke("list_watchlist");
        if (Array.isArray(wl)) {
          for (const w of wl.slice(0, 10)) {
            try {
              const lu: any[] = await invoke("get_lockup_schedule", { stockCode: w.stockCode });
              if (Array.isArray(lu)) {
                for (const l of lu) {
                  items.push({
                    type: "lockup",
                    code: w.stockCode,
                    name: w.stockName,
                    date: l.unlockDate ?? l.unlock_date ?? "",
                    detail: `${(l.unlockRatio ?? l.unlock_ratio ?? 0).toFixed(1)}% ${
                      t("stockAnalysis.settings.panels.lockup")
                    }`,
                  });
                }
              }
            } catch { /* */ }
          }
        }
      } catch { /* */ }
      // 分红除权
      try {
        const wl2: any[] = await invoke("list_watchlist");
        if (Array.isArray(wl2)) {
          for (const w of wl2.slice(0, 5)) {
            try {
              const dv: any[] = await invoke("get_dividend_records", { stockCode: w.stockCode });
              if (Array.isArray(dv)) {
                for (const d of dv.slice(0, 2)) {
                  const ex = d.exDate ?? d.ex_date ?? "";
                  if (ex) {
                    items.push({
                      type: "dividend",
                      code: w.stockCode,
                      name: w.stockName,
                      date: ex,
                      detail: `${(d.dividendPerShare ?? d.dividend_per_share ?? 0).toFixed(2)}${
                        t("stockAnalysis.settings.panels.perShare")
                      }`,
                    });
                  }
                }
              }
            } catch { /* */ }
          }
        }
      } catch { /* */ }
    } catch { /* */ }

    items.sort((a, b) => a.date.localeCompare(b.date));
    setEvents(items.slice(0, 30));
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

  return (
    <Card
      size="small"
      title={`📅 ${t("stockAnalysis.settings.panels.eventCalendar")}`}
      styles={{ body: { padding: "4px 8px" } }}
      extra={
        <Button size="small" loading={loading} onClick={load}>{t("stockAnalysis.settings.panels.refresh")}</Button>
      }
    >
      {loading
        ? <Spin size="small" />
        : events.length === 0
        ? <Empty image={Empty.PRESENTED_IMAGE_SIMPLE} description={t("stockAnalysis.settings.panels.noEvents")} />
        : (
          <List
            size="small"
            dataSource={events}
            renderItem={(ev) => (
              <List.Item
                style={{ cursor: "pointer", padding: "3px 0" }}
                onClick={() => analyze(ev.code)}
                actions={[
                  <Tag key="type" color={ev.type === "lockup" ? "orange" : "blue"} className="text-xs m-0">
                    {ev.type === "lockup"
                      ? t("stockAnalysis.settings.panels.lockup")
                      : t("stockAnalysis.settings.panels.dividend")}
                  </Tag>,
                ]}
              >
                <div className="flex items-center gap-2 text-xs w-full">
                  <Tag className="m-0 text-xs">{ev.code}</Tag>
                  <span className="flex-1 truncate">{ev.name}</span>
                  <span className="text-gray-400">{ev.date}</span>
                  <span className="text-gray-500">{ev.detail}</span>
                </div>
              </List.Item>
            )}
          />
        )}
    </Card>
  );
}
