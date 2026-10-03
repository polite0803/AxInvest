// i18n-exempt: 业务逻辑/格式化（金额、百分比、时间）字符串，非 UI 展示文本
import { List } from "@/components/common/AntdList";
import { formatYi } from "@/lib/format";
import { invoke } from "@/lib/invoke";
import { useStockAnalysisStore } from "@/stores";
import { Button, Card, Spin, Tag } from "antd";
import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { PanelEmpty, type PanelEmptyKind } from "./PanelEmpty";
import { useStockAnalysisPage } from "./StockAnalysisPageContext";
import { checkVendorEnabled, PANEL_VENDORS } from "./vendorCheck";

/**
 * 与后端 DTO 逐字段对齐（`astock-data::types::LimitUpPool*`，serde camelCase）。
 *
 * ⚠ 可空字段保持可空：后端把「接口没给这个数」留成 `null`（实测 `open_num` 约 2/3 的行是
 * `null`），前端不得折成 0 显示 —— 那会把「没说」伪装成「说是零」。
 */
interface LimitUpEntry {
  stockCode: string;
  stockName: string;
  /** 连板数（`high_days_value` 高 16 位） */
  limitUpStreak: number | null;
  /** 涨停天数（低 16 位）；「3天2板」= days 3 / streak 2，与连板数不同轴 */
  limitUpDays: number | null;
  turnoverRate: number | null;
  changePct: number | null;
  sealAmount: number | null;
  sealVolume: number | null;
  /** 炸板次数 */
  breakCount: number | null;
  firstSealTime: string | null;
  lastSealTime: string | null;
  /** 板型：一字板 / 换手板 / T字板（vendor 原文，不翻译） */
  limitUpType: string | null;
  reSealed: boolean | null;
  floatMarketCap: number | null;
  latestPrice: number | null;
  reasonTags: string[];
}

interface LimitUpBreadth {
  limitUpCount: number;
  touchedCount: number | null;
  /** 封板率（0-1，接口 `limit_up_count.today.rate`） */
  sealRate: number | null;
  breakCount: number | null;
  limitDownCount: number | null;
}

interface LimitUpPool {
  /** 接口自报的生效日期（实测等于请求日，不等时后端已直接报错） */
  poolDate: string;
  requestedDate: string | null;
  entries: LimitUpEntry[];
  breadth: LimitUpBreadth | null;
}

interface LimitUpPanelProps {
  /** 是否显示外边框(ScreenerPage Collapse 内嵌时传 false) */
  bordered?: boolean;
}

export function LimitUpPanel({ bordered = true }: LimitUpPanelProps = {}) {
  const { t } = useTranslation();
  const { openDataSourceSettings } = useStockAnalysisPage();
  const getStockQuote = useStockAnalysisStore((s) => s.getStockQuote);
  const getStockKline = useStockAnalysisStore((s) => s.getStockKline);
  const startAnalysis = useStockAnalysisStore((s) => s.startAnalysis);
  const [pool, setPool] = useState<LimitUpPool | null>(null);
  const [loading, setLoading] = useState(false);
  const [emptyKind, setEmptyKind] = useState<PanelEmptyKind | null>(null);
  const [emptyVendors, setEmptyVendors] = useState<string[] | undefined>(undefined);

  // 连板数优先（妖股初筛视角），同连板按换手降序
  const entries = [...(pool?.entries ?? [])].sort(
    (a, b) => (b.limitUpStreak ?? 0) - (a.limitUpStreak ?? 0) || (b.turnoverRate ?? 0) - (a.turnoverRate ?? 0),
  );

  const load = useCallback(async (silent = false) => {
    setLoading(true);
    setEmptyKind(null);
    setEmptyVendors(undefined);
    try {
      const check = await checkVendorEnabled("limitup", { silent });
      if (check.status === "disabled") {
        setPool(null);
        setEmptyKind("vendorDisabled");
        setEmptyVendors(check.vendors);
        return;
      }
      if (check.status === "backend_offline") {
        setPool(null);
        setEmptyKind("backendOffline");
        return;
      }
      const p = await invoke<LimitUpPool | null>("get_limit_up_pool");
      // 后端把「该日不可得」报成错误（走 catch），到这里的数据都属于请求的那一天
      if (!p) {
        setPool(null);
        setEmptyKind("noData");
        return;
      }
      setPool(p);
      if (p.entries.length === 0) { setEmptyKind("noData"); }
    } catch {
      setPool(null);
      setEmptyKind("connectionFailed");
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void load(true);
  }, [load]);

  const analyze = async (code: string) => {
    await getStockQuote(code);
    await getStockKline(code, "daily", 120);
    startAnalysis(code);
  };

  const breadth = pool?.breadth ?? null;

  return (
    <Card
      size="small"
      bordered={bordered}
      title={`🏆 ${t("stockAnalysis.settings.panels.limitUp")}`}
      styles={{ body: { padding: "4px 8px" } }}
      extra={
        <Button size="small" loading={loading} onClick={() => load()}>
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
            vendorNames={emptyVendors ?? PANEL_VENDORS.limitup}
            description={emptyKind === "noData" ? t("stockAnalysis.settings.panels.noLimitUp") : undefined}
            onOpenSettings={openDataSourceSettings}
          />
        )
        : (
          <>
            {pool && (
              <div className="text-xs text-gray-500 px-1 pb-1">
                {t("stockAnalysis.settings.panels.poolSummary", {
                  date: pool.poolDate,
                  count: breadth?.limitUpCount ?? pool.entries.length,
                  rate: breadth?.sealRate != null ? (breadth.sealRate * 100).toFixed(1) : "—",
                  break: breadth?.breakCount ?? "—",
                })}
              </div>
            )}
            <List
              size="small"
              dataSource={entries}
              renderItem={(s) => {
                const streak = s.limitUpStreak ?? 0;
                const days = s.limitUpDays ?? 0;
                const breaks = s.breakCount ?? 0;
                return (
                  <List.Item
                    style={{ cursor: "pointer", padding: "3px 0" }}
                    onClick={() => analyze(s.stockCode)}
                    actions={[
                      streak >= 2 && (
                        <Tag key="streak" color="volcano" className="text-xs m-0">
                          {days === streak
                            ? t("stockAnalysis.settings.panels.consecutive", { n: streak })
                            : t("stockAnalysis.settings.panels.boardDays", { days, boards: streak })}
                        </Tag>
                      ),
                      breaks > 0 && (
                        <Tag key="break" color="orange" className="text-xs m-0">
                          {t("stockAnalysis.settings.panels.breakTimes", { n: breaks })}
                        </Tag>
                      ),
                    ].filter(Boolean)}
                  >
                    <div className="flex items-center gap-2 text-xs w-full">
                      <Tag className="m-0 text-xs">{s.stockCode}</Tag>
                      <span className="flex-1 truncate">{s.stockName}</span>
                      {s.limitUpType && <span className="text-gray-400">{s.limitUpType}</span>}
                      <span className="font-mono">{s.latestPrice != null ? s.latestPrice.toFixed(2) : "—"}</span>
                      <span className="text-red-500">
                        {(s.changePct ?? 0) >= 0 ? "+" : ""}
                        {(s.changePct ?? 0).toFixed(1)}%
                      </span>
                      <span className="text-gray-400">
                        {t("stockAnalysis.settings.panels.turnover")} {(s.turnoverRate ?? 0).toFixed(1)}%
                      </span>
                      {s.sealAmount != null && (
                        <span className="text-gray-400">
                          {t("stockAnalysis.settings.panels.sealAmount")} {formatYi(s.sealAmount)}
                        </span>
                      )}
                    </div>
                  </List.Item>
                );
              }}
            />
          </>
        )}
    </Card>
  );
}
