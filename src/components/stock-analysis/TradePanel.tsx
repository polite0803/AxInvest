import { invoke } from "@/lib/invoke";
import { PlusOutlined } from "@ant-design/icons";
import { Button, Card, Input, InputNumber, message, Select, Switch, Table, Tag } from "antd";
import dayjs from "dayjs";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

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
  const [enabled, setEnabled] = useState(false);
  const [trades, setTrades] = useState<TradeRecord[]>([]);
  const [positions, setPositions] = useState<PositionSummary[]>([]);
  const [loading, setLoading] = useState(false);
  const [lastAnalysis, setLastAnalysis] = useState<{ action: string; targetPrice: number | null } | null>(null);
  const [form, setForm] = useState({
    stockCode: "",
    stockName: "",
    direction: "buy",
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
    { title: t("trade.stockCode"), dataIndex: "stockCode", width: 70 },
    { title: t("trade.shares"), dataIndex: "totalShares", width: 50 },
    {
      title: t("trade.cost"),
      dataIndex: "avgCost",
      width: 60,
      render: (v: number) => v.toFixed(2),
    },
    {
      title: t("trade.pnl"),
      dataIndex: "unrealizedPnl",
      width: 60,
      render: (v: number | null) =>
        v != null
          ? (
            <span style={{ color: v >= 0 ? "#3fb950" : "#f85149" }}>
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
      width: 30,
      render: (v: string) => (
        <Tag color={v === "buy" ? "green" : "red"}>
          {v === "buy" ? t("trade.buy") : t("trade.sell")}
        </Tag>
      ),
    },
    { title: t("trade.stockCode"), dataIndex: "stockCode", width: 60 },
    {
      title: t("trade.price"),
      dataIndex: "price",
      width: 55,
      render: (v: number) => v.toFixed(2),
    },
    { title: t("trade.quantity"), dataIndex: "quantity", width: 50 },
    {
      title: t("trade.pnl"),
      dataIndex: "realizedPnl",
      width: 60,
      render: (v: number | null) =>
        v != null
          ? (
            <span style={{ color: v >= 0 ? "#3fb950" : "#f85149" }}>
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
      <Card size="small" title={t("trade.title")}>
        <div
          className="text-xs text-center"
          style={{
            color: "var(--color-text-secondary)",
            padding: 12,
          }}
        >
          <p>{t("trade.disabledHint")}</p>
          <Switch
            checked={enabled}
            onChange={handleToggle}
            style={{ marginTop: 8 }}
          />
        </div>
      </Card>
    );
  }

  return (
    <Card
      size="small"
      title={
        <div className="flex justify-between items-center">
          <span>{t("trade.title")}</span>
          <Switch size="small" checked={enabled} onChange={handleToggle} />
        </div>
      }
    >
      {/* 录入表单 */}
      <div className="flex flex-col gap-1 mb-2">
        <div className="flex gap-1">
          <Input
            size="small"
            placeholder={t("trade.stockCode")}
            value={form.stockCode}
            onChange={(e) => setForm({ ...form, stockCode: e.target.value })}
            style={{ width: 90 }}
          />
          <Input
            size="small"
            placeholder={t("trade.stockName")}
            value={form.stockName}
            onChange={(e) => setForm({ ...form, stockName: e.target.value })}
            style={{ width: 80 }}
          />
          <Select
            size="small"
            value={form.direction}
            onChange={(v) => setForm({ ...form, direction: v })}
            options={[
              { value: "buy", label: t("trade.buy") },
              { value: "sell", label: t("trade.sell") },
            ]}
            style={{ width: 60 }}
          />
        </div>
        <div className="flex gap-1">
          <InputNumber
            size="small"
            placeholder={t("trade.price")}
            value={form.price}
            onChange={(v) => setForm({ ...form, price: v || 0 })}
            style={{ width: 100 }}
          />
          <InputNumber
            size="small"
            placeholder={t("trade.quantity")}
            value={form.quantity}
            onChange={(v) => setForm({ ...form, quantity: v || 100 })}
            step={100}
            style={{ width: 100 }}
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

      {/* 分析一致性提示 */}
      {lastAnalysis && (
        <div className="text-xs p-1 rounded" style={{ background: "var(--color-bg-elevated)", marginTop: 4 }}>
          <span style={{ color: "var(--color-text-secondary)" }}>最近分析:</span>
          <Tag color={lastAnalysis.action === "买入" ? "green" : lastAnalysis.action === "卖出" ? "red" : "blue"}>
            {lastAnalysis.action}
          </Tag>
          {lastAnalysis.targetPrice && <span>目标¥{lastAnalysis.targetPrice}</span>}
        </div>
      )}

      {/* 持仓汇总 */}
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
            const riskColor = maxPct > 50 ? "#f85149" : maxPct > 30 ? "#d29922" : "#3fb950";
            return (
              <div
                className="text-xs flex justify-between p-1 mt-1 rounded"
                style={{ background: "var(--color-bg-elevated)" }}
              >
                <span>
                  总市值: <b>{(totalMv / 10000).toFixed(1)}万</b>
                </span>
                <span style={{ color: totalPnl >= 0 ? "#3fb950" : "#f85149" }}>
                  浮动盈亏: <b>{totalPnl >= 0 ? "+" : ""}{totalPnl.toFixed(0)}</b>
                </span>
                <span style={{ color: riskColor }}>
                  集中度: <b>{maxPct.toFixed(0)}%</b>
                </span>
                <span>
                  持仓: <b>{positions.length}只</b>
                </span>
              </div>
            );
          })()}
        </>
      )}

      {/* 最近交易 */}
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
