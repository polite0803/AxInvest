/**
 * 核心接口日级健康记录（2026-10-28）。
 *
 * 数据来源：`get_interface_health_history`（每日收盘后由 `start_daily_snapshot_sweep`
 * 的同一 tick 幂等写入）。这里回答的是「**某一天**这条核心取数通道到底有没有给出数据」，
 * 与同页的 vendor 30s 实时健康、换源记录是三个不同事实，不得互相顶替。
 *
 * 三态缺席各占一句：归档未启用（没留存）/ 尚未跑过（没检查）/ 有记录。
 */
import { invoke } from "@/lib/invoke";
import type { DataStatus } from "@/types";
import { Card, Spin, Table, Tag, Tooltip } from "antd";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

interface InterfaceHealthRow {
  method: string;
  status: DataStatus;
  detail?: string;
  elapsedMs: number;
  rows: number;
}

interface InterfaceHealthReport {
  checkedAt: string;
  anchorDate: string;
  probeCode: string;
  rows: InterfaceHealthRow[];
}

interface InterfaceHealthHistory {
  storeEnabled: boolean;
  days: InterfaceHealthReport[];
}

const STATUS_KEYS: Record<DataStatus, string> = {
  // 好/故障/改版复用既有文案（`empty.sourceFailed` 与面板空态同一句，避免两套说法）
  ok: "stockAnalysis.settings.vendor.status.healthy",
  empty_by_time: "stockAnalysis.settings.interfaceHealth.statusEmptyByTime",
  source_failed: "stockAnalysis.empty.sourceFailed",
  schema_changed: "stockAnalysis.empty.schemaChanged",
};

const STATUS_COLORS: Record<DataStatus, string> = {
  ok: "green",
  empty_by_time: "default",
  source_failed: "red",
  schema_changed: "orange",
};

export function InterfaceHealthPanel() {
  const { t } = useTranslation();
  const [history, setHistory] = useState<InterfaceHealthHistory | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    invoke<unknown>("get_interface_health_history", { days: 7 })
      .then((raw) => {
        if (cancelled) { return; }
        // 形状不认识 = 本环境不提供该命令（浏览器 mock 回 `{}`），不是「没有记录」。
        if (
          raw && typeof raw === "object" && typeof (raw as { storeEnabled?: unknown }).storeEnabled
            === "boolean"
          && Array.isArray((raw as { days?: unknown }).days)
        ) {
          setHistory(raw as InterfaceHealthHistory);
          setError(null);
        } else {
          setHistory(null);
          setError(t("stockAnalysis.settings.interfaceHealth.unavailable"));
        }
      })
      .catch(() => {
        if (!cancelled) {
          setHistory(null);
          setError(t("stockAnalysis.settings.interfaceHealth.unavailable"));
        }
      })
      .finally(() => {
        if (!cancelled) { setLoading(false); }
      });
    return () => {
      cancelled = true;
    };
  }, [t]);

  const body = (() => {
    if (loading && !history) {
      return (
        <div className="flex justify-center py-6">
          <Spin size="small" />
        </div>
      );
    }
    if (error) {
      return <div className="text-xs text-red-400">{error}</div>;
    }
    if (!history) { return null; }
    if (!history.storeEnabled) {
      return (
        <div className="text-xs text-gray-400">
          {t("stockAnalysis.settings.interfaceHealth.storeDisabled")}
        </div>
      );
    }
    if (history.days.length === 0) {
      return (
        <div className="text-xs text-gray-400">
          {t("stockAnalysis.settings.interfaceHealth.empty")}
        </div>
      );
    }
    return (
      <Table<InterfaceHealthReport>
        size="small"
        rowKey={(r) => r.anchorDate}
        dataSource={history.days}
        pagination={false}
        columns={[
          {
            title: t("stockAnalysis.settings.interfaceHealth.colDate"),
            dataIndex: "anchorDate",
            width: 110,
            render: (v: string, r) => (
              <Tooltip title={`${t("stockAnalysis.settings.interfaceHealth.checkedAt")}: ${r.checkedAt}`}>
                <span>{v}</span>
              </Tooltip>
            ),
          },
          {
            title: t("stockAnalysis.settings.interfaceHealth.colSummary"),
            key: "summary",
            width: 150,
            render: (_: unknown, r) => {
              const ok = r.rows.filter((row) => row.status === "ok").length;
              return (
                <span className="text-xs">
                  {t("stockAnalysis.settings.interfaceHealth.summary", {
                    ok,
                    total: r.rows.length,
                  })}
                </span>
              );
            },
          },
          {
            title: t("stockAnalysis.settings.interfaceHealth.colIssues"),
            key: "issues",
            render: (_: unknown, r) => {
              const bad = r.rows.filter((row) => row.status !== "ok");
              if (bad.length === 0) {
                return <span className="text-xs text-green-500">{t(STATUS_KEYS.ok)}</span>;
              }
              return (
                <div className="flex flex-wrap gap-1">
                  {bad.map((row) => (
                    <Tooltip
                      key={row.method}
                      title={[`${row.elapsedMs}ms`, row.detail].filter(Boolean).join(" · ")}
                    >
                      <Tag color={STATUS_COLORS[row.status]} className="text-[10px] m-0">
                        {row.method} · {t(STATUS_KEYS[row.status])}
                      </Tag>
                    </Tooltip>
                  ))}
                </div>
              );
            },
          },
        ]}
      />
    );
  })();

  return (
    <Card size="small" title={t("stockAnalysis.settings.interfaceHealth.title")}>
      {body}
    </Card>
  );
}
