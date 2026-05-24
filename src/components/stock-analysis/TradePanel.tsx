import { invoke } from "@/lib/invoke";
import { useStockAnalysisStore } from "@/stores";
import { PlusOutlined } from "@ant-design/icons";
import { Button, Card, Input, InputNumber, message, Select, Switch, Table, Tag } from "antd";
import dayjs from "dayjs";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

/** 后端返回的分析动作常量（用于比较，不做 UI 展示） */
const A = { BUY: "买入", SELL: "卖出" } as const;

interface TradeRecord {
  id: string;
  stockCode: string;
  stockName: string;
  direction: string;
  price: number;
  quantity: number;
  tradeDate: string;
  tradeTime: string;
  realizedPnl: number | null;
  notes: string | null;
  createdAt: number;
}

interface PositionSummary {
  stockCode: string;
  stockName: string;
  totalShares: number;
  avgCost: number;
  currentPrice: number | null;
  marketValue: number | null;
  unrealizedPnl: number | null;
  unrealizedPnlPct: number | null;
  totalRealizedPnl: number;
}

export function TradePanel() {
  const { t } = useTranslation();
  const storeStockCode = useStockAnalysisStore((s) => s.stockCode);
  const storeStockName = useStockAnalysisStore((s) => s.stockName);
  const storeDecision = useStockAnalysisStore((s) => s.decision);
  const [enabled, setEnabled] = useState(false);
  const [trades, setTrades] = useState<TradeRecord[]>([]);
  const [positions, setPositions] = useState<PositionSummary[]>([]);
  const [loading, setLoading] = useState(false);
  const [lastAnalysis, setLastAnalysis] = useState<{ action: string; targetPrice: number | null } | null>(null);
  const [form, setForm] = useState({
    stockCode: "",
    stockName: "",
    direction: "buy" as string,
    price: 0,
    quantity: 100,
    notes: "",
  });

  const loadData = async () => {
    setLoading(true);
    try {
      const [t, p] = await Promise.all([
        invoke<TradeRecord[]>("list_trades", { stockCode: null, limit: 50 }),
        invoke<PositionSummary[]>("get_trade_positions"),
      ]);
      setTrades(t);
      setPositions(p);
    } catch {
      // 静默处理
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    if (enabled) {
      loadData();
    }
  }, [enabled]);

  // 自动从分析页同步股票代码/名称
  useEffect(() => {
    if (storeStockCode && !form.stockCode) {
      const dir = storeDecision?.action === A.SELL ? "sell" : "buy";
      setForm((f) => ({ ...f, stockCode: storeStockCode, stockName: storeStockName, direction: dir }));
    }
  }, [storeStockCode, storeStockName, storeDecision]);

  // 当股票代码变化时，获取最近分析决策
  useEffect(() => {
    if (form.stockCode && enabled) {
      invoke<{ stockCode: string; decisionJson: string | null }[]>("list_stock_analyses", { limit: 1 })
        .then((list) => {
          if (list.length > 0 && list[0].decisionJson) {
            try {
              const d = JSON.parse(list[0].decisionJson);
              setLastAnalysis({ action: d.action, targetPrice: d.targetPrice ?? null });
            } catch {
              setLastAnalysis(null);
            }
          }
        })
        .catch(() => setLastAnalysis(null));
    }
  }, [form.stockCode, enabled]);

  const handleRecord = async () => {
    if (!form.stockCode || !form.stockName || form.price <= 0) {
      message.warning(t("trade.fillRequired"));
      return;
    }
    const now = dayjs();
    try {
      await invoke("record_trade", {
        stockCode: form.stockCode,
        stockName: form.stockName,
        direction: form.direction,
        price: form.price,
        quantity: form.quantity,
        tradeDate: now.format("YYYY-MM-DD"),
        tradeTime: now.format("HH:mm"),
        notes: form.notes || null,
      });
      message.success(t("trade.recorded"));
      loadData();
    } catch (e) {
      message.error(String(e));
    }
  };

  const handleToggle = async (val: boolean) => {
    try {
      await invoke("toggle_trading_enabled", { enabled: val });
      setEnabled(val);
    } catch {
      message.error(t("trade.toggleFailed"));
    }
  };

  const positionColumns = [
    { title: t("trade.stockCode"), dataIndex: "stockCode", width: 56 },
    { title: t("trade.shares"), dataIndex: "totalShares", width: 44 },
    {
      title: t("trade.cost"),
      dataIndex: "avgCost",
      width: 50,
      render: (v: number) => v.toFixed(2),
    },
    {
      title: t("trade.pnl"),
      dataIndex: "unrealizedPnl",
      width: 50,
      render: (v: number | null) =>
        v != null
          ? (
            <span style={{ color: v >= 0 ? "var(--sa-green)" : "var(--sa-red)" }}>
              {v.toFixed(0)}
            </span>
          )
          : (
            "-"
          ),
    },
  ];

  const tradeColumns = [
    {
      title: "",
      dataIndex: "direction",
      width: 24,
      render: (v: string) => (
        <Tag color={v === "buy" ? "green" : "red"} style={{ fontSize: 10, lineHeight: "16px", padding: "0 4px" }}>
          {v === "buy" ? "买" : "卖"}
        </Tag>
      ),
    },
    { title: t("trade.stockCode"), dataIndex: "stockCode", width: 54 },
    {
      title: t("trade.price"),
      dataIndex: "price",
      width: 50,
      render: (v: number) => v.toFixed(2),
    },
    { title: t("trade.quantity"), dataIndex: "quantity", width: 44 },
    {
      title: t("trade.pnl"),
      dataIndex: "realizedPnl",
      width: 50,
      render: (v: number | null) =>
        v != null
          ? (
            <span style={{ color: v >= 0 ? "var(--sa-green)" : "var(--sa-red)" }}>
              {v.toFixed(0)}
            </span>
          )
          : (
            "-"
          ),
    },
  ];

  if (!enabled) {
    return (
      <Card size="small" title={t("trade.title")} styles={{ body: { padding: "8px 12px" } }}>
        <div
          className="flex items-center justify-between gap-2 text-xs"
          style={{ color: "var(--muted)" }}
        >
          <span>{t("trade.disabledHint")}</span>
          <Switch checked={enabled} onChange={handleToggle} />
        </div>
      </Card>
    );
  }

  return (
    <Card
      size="small"
      styles={{ body: { padding: "8px 10px" } }}
      title={
        <div className="flex justify-between items-center">
          <span>{t("trade.title")}</span>
          <Switch size="small" checked={enabled} onChange={handleToggle} />
        </div>
      }
    >
      {/* Entry form — 两行紧凑布局，适配侧栏 260px */}
      <div className="flex flex-col gap-1 mb-2">
        <div className="flex gap-1 flex-wrap">
          <Input
            size="small"
            placeholder={t("trade.stockCode")}
            value={form.stockCode}
            onChange={(e) => setForm({ ...form, stockCode: e.target.value })}
            style={{ width: 72 }}
          />
          <Input
            size="small"
            placeholder={t("trade.stockName")}
            value={form.stockName}
            onChange={(e) => setForm({ ...form, stockName: e.target.value })}
            style={{ width: 72 }}
          />
          <Select
            size="small"
            value={form.direction}
            onChange={(v) => setForm({ ...form, direction: v })}
            options={[
              { value: "buy", label: "买" },
              { value: "sell", label: "卖" },
            ]}
            style={{ width: 50 }}
          />
        </div>
        <div className="flex gap-1">
          <InputNumber
            size="small"
            placeholder={t("trade.price")}
            value={form.price}
            onChange={(v) => setForm({ ...form, price: v || 0 })}
            style={{ width: 82 }}
            min={0}
            step={0.01}
          />
          <InputNumber
            size="small"
            placeholder={t("trade.quantity")}
            value={form.quantity}
            onChange={(v) => setForm({ ...form, quantity: v || 100 })}
            step={100}
            min={100}
            style={{ width: 82 }}
          />
          <Button
            size="small"
            type="primary"
            icon={<PlusOutlined />}
            loading={loading}
            onClick={handleRecord}
          />
        </div>
      </div>

      {/* Analysis consistency hint */}
      {lastAnalysis && (
        <div className="text-xs p-1 rounded" style={{ background: "var(--surface)", marginTop: 4 }}>
          <span style={{ color: "var(--muted)" }}>{t("stockAnalysis.recentAnalysis")}:</span>
          <Tag color={lastAnalysis.action === A.BUY ? "green" : lastAnalysis.action === A.SELL ? "red" : "blue"}>
            {lastAnalysis.action}
          </Tag>
          {lastAnalysis.targetPrice && (
            <span>{t("stockAnalysis.targetPriceNote", { price: lastAnalysis.targetPrice })}</span>
          )}
        </div>
      )}

      {/* Position summary */}
      {positions.length > 0 && (
        <>
          <Table
            size="small"
            dataSource={positions}
            rowKey="stockCode"
            pagination={false}
            columns={positionColumns}
          />
          {(() => {
            const totalMv = positions.reduce((s, p) => s + (p.marketValue ?? 0), 0);
            const totalPnl = positions.reduce((s, p) => s + (p.unrealizedPnl ?? 0), 0);
            const maxPct = positions.length > 0 && totalMv > 0
              ? Math.max(...positions.map(p => ((p.marketValue ?? 0) / totalMv) * 100))
              : 0;
            const riskColor = maxPct > 50 ? "var(--sa-red)" : maxPct > 30 ? "var(--sa-amber)" : "var(--sa-green)";
            return (
              <div
                className="text-xs grid grid-cols-2 gap-x-2 p-1 mt-1 rounded"
                style={{ background: "var(--surface)" }}
              >
                <span>
                  {t("stockAnalysis.totalMarketValue")}:{" "}
                  <b>{(totalMv / 10000).toFixed(1)}{t("stockAnalysis.wanUnit")}</b>
                </span>
                <span style={{ color: totalPnl >= 0 ? "var(--sa-green)" : "var(--sa-red)" }}>
                  {t("stockAnalysis.unrealizedPnl")}: <b>{totalPnl >= 0 ? "+" : ""}{totalPnl.toFixed(0)}</b>
                </span>
                <span style={{ color: riskColor }}>
                  {t("stockAnalysis.concentration")}: <b>{maxPct.toFixed(0)}%</b>
                </span>
                <span>
                  {t("stockAnalysis.holdings")}: <b>{positions.length}{t("stockAnalysis.sharesUnit")}</b>
                </span>
              </div>
            );
          })()}
        </>
      )}

      {/* Recent trades */}
      {trades.length > 0 && (
        <Table
          size="small"
          dataSource={trades.slice(0, 10)}
          rowKey="id"
          pagination={false}
          className="mt-1"
          columns={tradeColumns}
        />
      )}
    </Card>
  );
}
