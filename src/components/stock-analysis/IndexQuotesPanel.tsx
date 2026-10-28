import { invoke } from "@/lib/invoke";
import type { DataOutcome, DataStatus, IndexQuote } from "@/types";
import { Button, Card, Spin, Table } from "antd";
import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { PanelEmpty, type PanelEmptyKind } from "./PanelEmpty";

const DATA_STATUSES: DataStatus[] = ["ok", "empty_by_time", "source_failed", "schema_changed"];

/**
 * 兼容两代载荷：新命令回 `{status,data,detail}` 信封，历史/mock 可能回裸数组。
 *
 * 形状不认识时**抛错**（→ `connectionFailed`），不把「读不懂」说成「没有数据」。
 */
function normalizeIndexQuotes(raw: unknown): DataOutcome<IndexQuote[]> {
  if (Array.isArray(raw)) {
    return { status: raw.length > 0 ? "ok" : "empty_by_time", data: raw as IndexQuote[] };
  }
  if (raw && typeof raw === "object" && Array.isArray((raw as { data?: unknown }).data)) {
    const obj = raw as { status?: unknown; data: IndexQuote[]; detail?: unknown };
    return {
      status: DATA_STATUSES.includes(obj.status as DataStatus)
        ? obj.status as DataStatus
        : "schema_changed",
      data: obj.data,
      detail: typeof obj.detail === "string" && obj.detail.trim() ? obj.detail.trim() : undefined,
    };
  }
  throw new Error("unexpected index quotes payload");
}

export function IndexQuotesPanel() {
  const { t } = useTranslation();
  const [quotes, setQuotes] = useState<IndexQuote[]>([]);
  const [loading, setLoading] = useState(false);
  const [emptyKind, setEmptyKind] = useState<PanelEmptyKind | null>(null);
  const [detail, setDetail] = useState<string | null>(null);

  const apply = useCallback((raw: unknown) => {
    const outcome = normalizeIndexQuotes(raw);
    setDetail(outcome.detail ?? null);
    if (outcome.status === "ok" && outcome.data.length > 0) {
      setQuotes(outcome.data);
      setEmptyKind(null);
      return;
    }
    setQuotes([]);
    // 四态各占一句：`empty_by_time` 是「该时点确实没有」，另两档是故障/改版。
    switch (outcome.status) {
      case "source_failed":
        setEmptyKind("sourceFailed");
        break;
      case "schema_changed":
        setEmptyKind("schemaChanged");
        break;
      default:
        setEmptyKind("noData");
        break;
    }
  }, []);

  const load = useCallback(async () => {
    setLoading(true);
    setEmptyKind(null);
    try {
      apply(await invoke<unknown>("get_index_quotes"));
    } catch {
      setQuotes([]);
      setDetail(null);
      setEmptyKind("connectionFailed");
    } finally {
      setLoading(false);
    }
  }, [apply]);

  useEffect(() => {
    void load();
  }, [load]);

  const columns = [
    { title: t("stockAnalysis.alert.name"), dataIndex: "name", key: "name", width: 100 },
    {
      title: t("stockAnalysis.price"),
      dataIndex: "price",
      key: "price",
      width: 80,
      render: (v: number | undefined) => v != null ? v.toFixed(2) : "-",
    },
    {
      title: t("stockAnalysis.change"),
      dataIndex: "changePct",
      key: "changePct",
      width: 80,
      render: (v: number | undefined) => {
        if (v == null) { return <span>-</span>; }
        const color = v >= 0 ? "var(--red)" : "var(--green)";
        return <span style={{ color, fontWeight: "bold" }}>{v >= 0 ? "+" : ""}{v.toFixed(2)}%</span>;
      },
    },
  ];

  // 有成因时把它接到空态文案后面（`PanelEmpty` 的 description 会覆盖默认文案，
  // 故这里自己拼一次，保证「哪种缺席 + 具体证据」同屏可见）。
  const emptyDescription = (() => {
    if (!emptyKind) { return undefined; }
    const base = emptyKind === "sourceFailed"
      ? t("stockAnalysis.empty.sourceFailed")
      : emptyKind === "schemaChanged"
      ? t("stockAnalysis.empty.schemaChanged")
      : emptyKind === "connectionFailed"
      ? t("stockAnalysis.settings.vendor.connectionFailed")
      : t("stockAnalysis.indexQuoteEmpty");
    return detail ? `${base}：${detail}` : base;
  })();

  return (
    <Card
      size="small"
      title={`📈 ${t("stockAnalysis.indexQuotes")}`}
      styles={{ body: { padding: 0 } }}
      extra={
        <Button size="small" loading={loading} onClick={load}>
          {t("stockAnalysis.settings.panels.refresh")}
        </Button>
      }
    >
      {loading
        ? <Spin size="small" style={{ display: "block", margin: "16px auto" }} />
        : emptyKind
        ? <PanelEmpty kind={emptyKind} description={emptyDescription} />
        : (
          <Table
            dataSource={quotes}
            columns={columns}
            rowKey={(r) => r.code ?? r.name}
            size="small"
            pagination={false}
          />
        )}
    </Card>
  );
}
