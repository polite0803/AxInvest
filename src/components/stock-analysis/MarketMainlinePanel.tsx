import { ReloadOutlined } from "@ant-design/icons";
import { Alert, Button, Card, Spin, Table, Tag } from "antd";
import type { ColumnsType } from "antd/es/table";
import { useCallback, useEffect, useState } from "react";
import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";

import { invoke } from "@/lib/invoke";

/** `market_mainlines` 的一行（后端实体 `#[serde(rename_all = "camelCase")]`）。 */
interface MarketMainlineRow {
  id: string;
  mainlineDate: string;
  theme: string;
  themeCategory: string;
  narrative: string;
  /** 落库是 JSON 字符串数组（`"[]"` 默认值），不是数组本体 */
  representativeSymbols: string;
  strengthScore: number;
  persistence: string;
  status: string;
  sourceWorkflowExecutionId: string | null;
}

const RECENT_DAYS = 7;

/** 代表标的列：解析失败要给数不出数（空数组与「存的不是 JSON 数组」在审计上是两回事）。 */
function symbolCount(raw: string): string {
  try {
    const parsed = JSON.parse(raw);
    return Array.isArray(parsed) ? String(parsed.length) : "—";
  } catch {
    return "—";
  }
}

/**
 * 市场主线读侧面板（D7，`PLAN-chain-closure-remediation.md` §3/§9）。
 *
 * 存在理由：`daily-market-events` 每天 18:00 都在跑，但落库全靠模型自愿调工具、实测近 8 次
 * 一次没调 ⇒ `market_mainlines` 恒 0 行；而读侧更早就断了 —— 十个 `market_mainline_*` 命令
 * 全都注册在案，**前端零调用者**，界面连「这张表是空的」都看不见。
 * A3 已经把写入变成图上的确定性节点，本面板补的是读侧那一半。
 *
 * ⚠ 本面板的「有数据」态要等**应用重启**（v2 模板随重启才重播种）并跑过一趟 18:00 才会出现；
 * 现在看到空态是预期的，不是取数坏了 —— 所以空态文案必须把这层说清楚（缺席要能归因）。
 */
export function MarketMainlinePanel() {
  const { t } = useTranslation();
  const [rows, setRows] = useState<MarketMainlineRow[] | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const data = await invoke<MarketMainlineRow[]>("market_mainline_list_recent", {
        days: RECENT_DAYS,
      });
      setRows(Array.isArray(data) ? data : []);
      setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
      setRows(null);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  const columns: ColumnsType<MarketMainlineRow> = [
    {
      title: t("backtest.mainlines.colDate"),
      dataIndex: "mainlineDate",
      key: "mainlineDate",
      width: 100,
    },
    {
      title: t("backtest.mainlines.colTheme"),
      dataIndex: "theme",
      key: "theme",
      render: (theme: string, row) => (
        <div>
          <strong>{theme}</strong>
          <div style={{ fontSize: 11, color: "var(--muted)" }}>{row.narrative}</div>
        </div>
      ),
    },
    {
      title: t("backtest.mainlines.colCategory"),
      dataIndex: "themeCategory",
      key: "themeCategory",
      width: 88,
    },
    {
      title: t("backtest.mainlines.colStrength"),
      dataIndex: "strengthScore",
      key: "strengthScore",
      width: 70,
      align: "right",
      render: (v: number) => <span data-testid="mainline-strength">{v.toFixed(0)}</span>,
    },
    {
      title: t("backtest.mainlines.colSymbols"),
      key: "symbols",
      width: 64,
      align: "right",
      render: (_, row) => <span data-testid="mainline-symbols">{symbolCount(row.representativeSymbols)}</span>,
    },
    {
      title: t("backtest.mainlines.colSource"),
      key: "source",
      width: 120,
      render: (_, row) => (
        // **不截断**来源执行 id：截前几位既没有区分度（同前缀），又让「按批次归因」在界面上做不到
        <Tag data-testid="mainline-source">{row.sourceWorkflowExecutionId ?? "—"}</Tag>
      ),
    },
  ];

  let body: ReactNode;
  if (loading) {
    body = <Spin size="small" style={{ display: "block", margin: "16px auto" }} />;
  } else if (error) {
    body = (
      <Alert
        type="error"
        showIcon
        className="text-xs!"
        data-testid="mainlines-failed"
        title={t("backtest.mainlines.stateFailed", { error })}
      />
    );
  } else if (!rows || rows.length === 0) {
    body = (
      <Alert
        type="warning"
        showIcon
        className="text-xs!"
        data-testid="mainlines-empty"
        title={<span className="text-xs">{t("backtest.mainlines.stateNoRows")}</span>}
      />
    );
  } else {
    body = (
      <Table
        size="small"
        rowKey="id"
        columns={columns}
        dataSource={rows}
        pagination={false}
        scroll={{ y: 260 }}
      />
    );
  }

  return (
    <Card
      size="small"
      title={t("backtest.mainlines.title", { days: RECENT_DAYS })}
      data-testid="mainlines-panel"
      styles={{ body: { padding: "8px 10px" } }}
      extra={
        <Button
          size="small"
          icon={<ReloadOutlined />}
          loading={loading}
          onClick={() => void load()}
          data-testid="mainlines-reload"
        >
          {t("common.refresh")}
        </Button>
      }
    >
      {body}
    </Card>
  );
}
