import { List } from "@/components/common/AntdList";
import { invoke } from "@/lib/invoke";
import { useStockAnalysisStore } from "@/stores";
import { Button, Card, Spin, Tag } from "antd";
import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { PanelEmpty, type PanelEmptyKind } from "./PanelEmpty";
import { useStockAnalysisPage } from "./StockAnalysisPageContext";
import { checkVendorEnabled, PANEL_VENDORS } from "./vendorCheck";

interface EventItem {
  type: "lockup" | "dividend";
  code: string;
  name: string;
  date: string;
  detail: string;
}

export function EventCalendarPanel() {
  const { t } = useTranslation();
  const { openDataSourceSettings } = useStockAnalysisPage();
  const stockCode = useStockAnalysisStore((s) => s.stockCode);
  const stockName = useStockAnalysisStore((s) => s.stockName);
  const getStockQuote = useStockAnalysisStore((s) => s.getStockQuote);
  const getStockKline = useStockAnalysisStore((s) => s.getStockKline);
  const startAnalysis = useStockAnalysisStore((s) => s.startAnalysis);
  const [events, setEvents] = useState<EventItem[]>([]);
  const [loading, setLoading] = useState(false);
  const [emptyKind, setEmptyKind] = useState<PanelEmptyKind | null>(null);
  const [emptyVendors, setEmptyVendors] = useState<string[] | undefined>(undefined);

  const fetchOneStock = useCallback(async (code: string, name: string, items: EventItem[]) => {
    try {
      const lu: any[] = await invoke("get_lockup_schedule", { stockCode: code });
      if (Array.isArray(lu)) {
        for (const l of lu.slice(0, 5)) {
          const date = l.unlockDate ?? l.unlock_date ?? "";
          if (!date) { continue; }
          items.push({
            type: "lockup",
            code,
            name,
            date,
            detail: `${(Number(l.unlockRatio ?? l.unlock_ratio ?? 0)).toFixed(1)}% ${
              t("stockAnalysis.settings.panels.lockup")
            }`,
          });
        }
      }
    } catch { /* 单只失败不影响其他 */ }
    try {
      const dv: any[] = await invoke("get_dividend_records", { stockCode: code });
      if (Array.isArray(dv)) {
        for (const d of dv.slice(0, 3)) {
          const ex = d.exDate ?? d.ex_date ?? "";
          if (!ex) { continue; }
          items.push({
            type: "dividend",
            code,
            name,
            date: ex,
            detail: `${(Number(d.dividendPerShare ?? d.dividend_per_share ?? 0)).toFixed(2)}${
              t("stockAnalysis.settings.panels.perShare")
            }`,
          });
        }
      }
    } catch { /* */ }
  }, [t]);

  const load = useCallback(async () => {
    setLoading(true);
    setEmptyKind(null);
    setEmptyVendors(undefined);
    const check = await checkVendorEnabled("events", { silent: true });
    if (check.status === "disabled") {
      setEvents([]);
      setEmptyKind("vendorDisabled");
      setEmptyVendors(check.vendors);
      setLoading(false);
      return;
    }
    if (check.status === "backend_offline") {
      setEvents([]);
      setEmptyKind("backendOffline");
      setLoading(false);
      return;
    }

    const items: EventItem[] = [];
    // 优先：当前正在分析的股票（即使没在自选里也能看到）
    if (stockCode) {
      await fetchOneStock(stockCode, stockName ?? stockCode, items);
    }
    // 补充：自选股列表
    try {
      const wl: any[] = await invoke("list_watchlist");
      if (Array.isArray(wl)) {
        for (const w of wl.slice(0, 8)) {
          if (stockCode && w.stockCode === stockCode) { continue; }
          await fetchOneStock(w.stockCode, w.stockName ?? w.stockCode, items);
        }
      }
    } catch { /* 无自选或后端不可用时跳过 */ }

    items.sort((a, b) => a.date.localeCompare(b.date));
    setEvents(items.slice(0, 30));
    if (items.length === 0) { setEmptyKind("noData"); }
    setLoading(false);
  }, [stockCode, stockName, fetchOneStock]);

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
        <Button size="small" loading={loading} onClick={load}>
          {t("stockAnalysis.settings.panels.refresh")}
        </Button>
      }
    >
      {loading
        ? <Spin size="small" style={{ display: "block", margin: "16px auto" }} />
        : emptyKind
        ? (
          <PanelEmpty
            kind={emptyKind}
            vendorNames={emptyVendors ?? PANEL_VENDORS.events}
            description={emptyKind === "noData"
              ? (!stockCode ? t("stockAnalysis.selectStockFirst") : t("stockAnalysis.settings.panels.noEvents"))
              : undefined}
            onOpenSettings={openDataSourceSettings}
          />
        )
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
