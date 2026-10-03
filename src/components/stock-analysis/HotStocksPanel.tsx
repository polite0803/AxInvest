import { invoke } from "@/lib/invoke";
import { useStockAnalysisStore } from "@/stores";
import { Button, Card, Spin, Table, Tag } from "antd";
import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { PanelEmpty, type PanelEmptyKind } from "./PanelEmpty";
import { useStockAnalysisPage } from "./StockAnalysisPageContext";
import { checkVendorEnabled, PANEL_VENDORS } from "./vendorCheck";

/**
 * 与后端 DTO 对齐（`astock-data::types::HotStock`，serde camelCase）。
 *
 * 名目收编(2026-10-03)：本面板此前读的是同花顺**涨停池**前 20 行（`get_hot_stocks`
 * 打的却是 `limit_up/limit_up_pool`），所以列里有「最新价」——那个字段**从来不在 DTO 里**，
 * 渲染恒为 `-`。现在后端给的是真热度榜，列改成榜内名次与热度值；
 * 换手率热股榜不提供（真值在涨停池面板那边），故不再占一列。
 */
interface HotStock {
  stockCode: string;
  stockName: string;
  changePct: number;
  turnoverRate: number | null;
  reasonTags: string[];
  sector: string | null;
  /** 榜内名次（1 起，实测榜长恒 100） */
  rank: number | null;
  /** 热度值（只有同期可比，量级随口径变化） */
  hotValue: number | null;
}

interface HotStocksPanelProps {
  /** 是否显示外边框(ScreenerPage Collapse 内嵌时传 false) */
  bordered?: boolean;
}

export function HotStocksPanel({ bordered = true }: HotStocksPanelProps = {}) {
  const { t } = useTranslation();
  const { openDataSourceSettings } = useStockAnalysisPage();
  const getStockQuote = useStockAnalysisStore((s) => s.getStockQuote);
  const getStockKline = useStockAnalysisStore((s) => s.getStockKline);
  const startAnalysis = useStockAnalysisStore((s) => s.startAnalysis);
  const [stocks, setStocks] = useState<HotStock[]>([]);
  const [loading, setLoading] = useState(false);
  const [emptyKind, setEmptyKind] = useState<PanelEmptyKind | null>(null);
  const [emptyVendors, setEmptyVendors] = useState<string[] | undefined>(undefined);

  const load = useCallback(async (silent = false) => {
    setLoading(true);
    setEmptyKind(null);
    setEmptyVendors(undefined);
    try {
      const check = await checkVendorEnabled("hotstocks", { silent });
      if (check.status === "disabled") {
        setStocks([]);
        setEmptyKind("vendorDisabled");
        setEmptyVendors(check.vendors);
        return;
      }
      if (check.status === "backend_offline") {
        setStocks([]);
        setEmptyKind("backendOffline");
        return;
      }
      const data = await invoke<HotStock[]>("get_hot_stocks");
      if (Array.isArray(data) && data.length > 0) {
        setStocks(data);
      } else {
        setStocks([]);
        setEmptyKind("noData");
      }
    } catch {
      setStocks([]);
      setEmptyKind("connectionFailed");
    } finally {
      setLoading(false);
    }
  }, []);

  // 首屏加载与「刷新」按钮走同一段逻辑（此前是 Promise 链抄了一遍，两处会各自漂移）
  useEffect(() => {
    void load(true);
  }, [load]);

  const analyze = async (code: string) => {
    await getStockQuote(code);
    await getStockKline(code, "daily", 120);
    startAnalysis(code);
  };

  const columns = [
    {
      title: t("stockAnalysis.settings.panels.rank"),
      dataIndex: "rank",
      key: "rank",
      width: 56,
      render: (v: number | null) => (v != null ? `#${v}` : "-"),
    },
    {
      title: t("stockAnalysis.alert.code"),
      dataIndex: "stockCode",
      key: "stockCode",
      width: 70,
      render: (code: string) => <Tag className="m-0 text-xs">{code}</Tag>,
    },
    {
      title: t("stockAnalysis.alert.name"),
      dataIndex: "stockName",
      key: "stockName",
      width: 80,
      render: (v: string | null) => v ?? "-",
    },
    {
      title: t("stockAnalysis.settings.panels.hotValue"),
      dataIndex: "hotValue",
      key: "hotValue",
      width: 80,
      render: (v: number | null) => (v != null ? v.toLocaleString() : "-"),
    },
    {
      title: t("stockAnalysis.change"),
      dataIndex: "changePct",
      key: "changePct",
      width: 70,
      render: (v: number | null | undefined) => {
        if (v == null) { return <span>-</span>; }
        const color = v >= 0 ? "var(--sa-red)" : "var(--sa-green)";
        return <span style={{ color, fontWeight: "bold" }}>{v >= 0 ? "+" : ""}{v.toFixed(2)}%</span>;
      },
    },
    {
      title: t("stockAnalysis.settings.panels.tags"),
      dataIndex: "reasonTags",
      key: "reasonTags",
      render: (tags: string[] | null) => (
        <div className="flex flex-wrap gap-0.5">
          {(tags ?? []).slice(0, 2).map((tag, i) => <Tag key={i} color="volcano" className="text-xs m-0">{tag}</Tag>)}
        </div>
      ),
    },
  ];

  return (
    <Card
      size="small"
      bordered={bordered}
      title={`🔥 ${t("stockAnalysis.settings.panels.hotStocks")}`}
      styles={{ body: { padding: 0 } }}
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
            vendorNames={emptyVendors ?? PANEL_VENDORS.hotstocks}
            description={emptyKind === "noData" ? t("stockAnalysis.settings.panels.noHot") : undefined}
            onOpenSettings={openDataSourceSettings}
          />
        )
        : (
          <Table
            dataSource={stocks}
            columns={columns}
            rowKey="stockCode"
            size="small"
            pagination={false}
            onRow={(record) => ({ onClick: () => analyze(record.stockCode), style: { cursor: "pointer" } })}
          />
        )}
    </Card>
  );
}
