// SPDX-License-Identifier: AGPL-3.0-only

import { translateBackendError } from "@/lib/errorI18n";
import { invoke } from "@/lib/invoke";
import type { MoverRecallBatchState, MoverRecallEventItem } from "@/types";
import { Alert, Descriptions, Drawer, Empty, Space, Spin, Table, Tag, Typography } from "antd";
import type { ColumnsType } from "antd/es/table";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { formatDateTime, formatDuration } from "../formatUtils";

interface MoverRecallBatchDrawerProps {
  /** 为空时 Drawer 关闭；切换 batchId 自动拉取明细 */
  batchId: string | null;
  onClose: () => void;
}

/** 批次运行中的轮询间隔（与 RunsTab 活跃轮询对齐）。 */
const ACTIVE_POLL_MS = 2000;

/**
 * 漏检补分析批次详情 Drawer：拉取单批次完整状态（含逐票 items），
 * running 状态 2s 轮询；列表行只有计数摘要，明细按 batchId 单取。
 */
export function MoverRecallBatchDrawer(
  { batchId, onClose }: MoverRecallBatchDrawerProps,
) {
  const { t, i18n } = useTranslation();
  const locale = i18n.language;
  const [data, setData] = useState<MoverRecallBatchState | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!batchId) {
      return;
    }
    let cancelled = false;
    let timer: number | undefined;

    const fetchOnce = async (first: boolean) => {
      if (first) {
        setLoading(true);
      }
      setError(null);
      if (first) {
        setData(null);
      }
      try {
        const batch = await invoke<MoverRecallBatchState | null>(
          "get_mover_recall_analysis_batch",
          { batchId },
        );
        if (cancelled) {
          return;
        }
        setData(batch);
        if (batch?.status === "running") {
          timer = window.setTimeout(() => void fetchOnce(false), ACTIVE_POLL_MS);
        }
      } catch (e: unknown) {
        if (!cancelled) {
          setError(translateBackendError(e));
        }
      } finally {
        if (first && !cancelled) {
          setLoading(false);
        }
      }
    };

    void fetchOnce(true);
    return () => {
      cancelled = true;
      if (timer !== undefined) {
        window.clearTimeout(timer);
      }
    };
  }, [batchId]);

  const columns: ColumnsType<MoverRecallEventItem> = [
    {
      title: t("tasksCenter.moverRecall.colStock"),
      key: "stock",
      width: 130,
      render: (_, r) => (
        <Space direction="vertical" size={0}>
          <Typography.Text strong style={{ fontSize: 12 }}>{r.stockCode}</Typography.Text>
          <Typography.Text type="secondary" style={{ fontSize: 11 }}>{r.stockName}</Typography.Text>
        </Space>
      ),
    },
    {
      title: t("tasksCenter.moverRecall.colPeriod"),
      dataIndex: "period",
      key: "period",
      width: 70,
      render: (v: string) => <Tag>{v}</Tag>,
    },
    {
      title: t("tasksCenter.moverRecall.colWindowStart"),
      dataIndex: "windowStart",
      key: "windowStart",
      width: 105,
      render: (v: string) => <Typography.Text style={{ fontSize: 12 }}>{v}</Typography.Text>,
    },
    {
      title: t("tasksCenter.moverRecall.colAnchor"),
      dataIndex: "anchorDate",
      key: "anchorDate",
      width: 105,
      render: (v: string) => <Typography.Text type="secondary" style={{ fontSize: 12 }}>{v}</Typography.Text>,
    },
    {
      title: t("tasksCenter.moverRecall.colCum"),
      dataIndex: "cumGainPct",
      key: "cumGainPct",
      width: 80,
      align: "right",
      render: (v: number) => <Typography.Text type="danger" style={{ fontSize: 12 }}>+{v.toFixed(1)}%</Typography.Text>,
    },
    {
      title: t("tasksCenter.moverRecall.colLayer"),
      dataIndex: "missLayer",
      key: "missLayer",
      width: 120,
      render: (v: string) => t(`stockAnalysis.moverRecall.layer.${v}`, { defaultValue: v }),
    },
    {
      title: t("tasksCenter.moverRecall.colStatus"),
      dataIndex: "status",
      key: "status",
      width: 80,
      render: (v: string) => {
        const color = v === "analyzed" ? "success" : v === "failed" ? "error" : "processing";
        const label = v === "analyzed"
          ? t("tasksCenter.moverRecall.statusAnalyzed")
          : v === "failed"
          ? t("tasksCenter.moverRecall.statusFailed")
          : t("tasksCenter.moverRecall.statusRunning");
        return <Tag color={color}>{label}</Tag>;
      },
    },
    {
      title: t("tasksCenter.moverRecall.colError"),
      key: "errorDetail",
      render: (_, r) =>
        r.errorDetail
          ? <Typography.Text type="danger" style={{ fontSize: 12 }}>{r.errorDetail}</Typography.Text>
          : r.analysisId
          ? <Typography.Text code style={{ fontSize: 11 }}>{r.analysisId}</Typography.Text>
          : "-",
    },
  ];

  return (
    <Drawer
      open={batchId !== null}
      onClose={onClose}
      title={t("tasksCenter.moverRecall.drawerTitle")}
      styles={{ wrapper: { width: 880 } }}
      destroyOnHidden
    >
      <Spin spinning={loading}>
        {error ? <Alert type="error" showIcon message={error} style={{ marginBottom: 12 }} /> : null}
        {!loading && !error && !data
          ? (
            <Empty
              image={Empty.PRESENTED_IMAGE_SIMPLE}
              description={t("tasksCenter.batchDetail.notFound")}
            />
          )
          : null}
        {data
          ? (
            <Space direction="vertical" size="middle" style={{ width: "100%" }}>
              <Descriptions
                size="small"
                bordered
                column={{ xs: 1, sm: 2 }}
                items={[
                  {
                    key: "status",
                    label: t("tasksCenter.column.status"),
                    children: (
                      <Tag color={statusColor(data.status)}>
                        {t(`tasksCenter.moverRecall.batchStatus.${data.status}`)}
                      </Tag>
                    ),
                  },
                  {
                    key: "counts",
                    label: t("tasksCenter.batchDetail.progress"),
                    children: (
                      <Space size={4} wrap>
                        <Tag color="success">{t("replayWorkbench.batch.okCount", { n: data.ok })}</Tag>
                        <Tag color="error">{t("replayWorkbench.batch.failCount", { n: data.fail })}</Tag>
                        <Tag>{t("replayWorkbench.batch.totalCount", { n: data.total })}</Tag>
                      </Space>
                    ),
                  },
                  {
                    key: "batchId",
                    label: "ID",
                    children: <Typography.Text code>{data.batchId}</Typography.Text>,
                  },
                  {
                    key: "created",
                    label: t("tasksCenter.history.started"),
                    children: formatDateTime(data.createdAtMs, locale),
                  },
                  {
                    key: "finished",
                    label: t("tasksCenter.batchDetail.finishedAt"),
                    children: data.finishedAtMs !== null
                      ? formatDateTime(data.finishedAtMs, locale)
                      : "-",
                  },
                  {
                    key: "duration",
                    label: t("tasksCenter.history.duration"),
                    children: data.finishedAtMs !== null
                      ? formatDuration(data.finishedAtMs - data.createdAtMs, locale)
                      : "-",
                  },
                  {
                    key: "message",
                    label: t("tasksCenter.batchDetail.message"),
                    children: data.message ?? "-",
                  },
                ]}
              />

              {data.errorDetail
                ? (
                  <Alert
                    type="error"
                    showIcon
                    title={t("tasksCenter.batchDetail.errorDetail")}
                    description={data.errorDetail}
                  />
                )
                : null}

              <Typography.Title level={5} style={{ marginBottom: 0 }}>
                {t("tasksCenter.moverRecall.itemsTitle", { n: data.items.length })}
              </Typography.Title>
              <Table<MoverRecallEventItem>
                size="small"
                pagination={{ pageSize: 10 }}
                rowKey="eventId"
                columns={columns}
                dataSource={data.items}
                scroll={{ y: 420 }}
                locale={{
                  emptyText: (
                    <Empty
                      image={Empty.PRESENTED_IMAGE_SIMPLE}
                      description={t("tasksCenter.moverRecall.itemsEmpty")}
                    />
                  ),
                }}
              />
            </Space>
          )
          : null}
      </Spin>
    </Drawer>
  );
}

function statusColor(status: MoverRecallBatchState["status"]): string {
  switch (status) {
    case "completed":
      return "success";
    case "partial":
      return "warning";
    case "failed":
      return "error";
    case "interrupted":
      return "default";
    case "running":
      return "processing";
  }
}
