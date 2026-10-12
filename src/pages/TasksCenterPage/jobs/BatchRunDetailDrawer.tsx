// SPDX-License-Identifier: AGPL-3.0-only

import { BatchStatusTag } from "@/components/time-travel/BatchStatusTag";
import { useBatchAnalysisColumns, useBatchHorizonColumns } from "@/components/time-travel/batchTables";
import { translateBackendError } from "@/lib/errorI18n";
import { invoke } from "@/lib/invoke";
import type { TimeTravelBatchState } from "@/types";
import { Alert, Descriptions, Drawer, Empty, Space, Spin, Table, Tag, Typography } from "antd";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { formatDateTime, formatDuration } from "../formatUtils";

interface BatchRunDetailDrawerProps {
  /** 为空时 Drawer 关闭；切换 batchId 自动拉取 get_time_travel_batch 明细 */
  batchId: string | null;
  onClose: () => void;
}

/**
 * 批次复盘历史详情 Drawer：拉取单批次完整状态（含 analyses / horizons /
 * serenityError / reflectionSummary），供任务中心手动作业 → 批次历史查看。
 * 列表行只有摘要计数，明细一律按 batchId 单取。
 */
export function BatchRunDetailDrawer({ batchId, onClose }: BatchRunDetailDrawerProps) {
  const { t, i18n } = useTranslation();
  const locale = i18n.language;
  const analysisColumns = useBatchAnalysisColumns();
  const horizonColumns = useBatchHorizonColumns();
  const [data, setData] = useState<TimeTravelBatchState | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!batchId) {
      return;
    }
    let cancelled = false;
    setLoading(true);
    setError(null);
    setData(null);
    invoke<TimeTravelBatchState | null>("get_time_travel_batch", { batchId })
      .then((batch) => {
        if (cancelled) {
          return;
        }
        setData(batch);
      })
      .catch((e: unknown) => {
        if (!cancelled) {
          setError(translateBackendError(e));
        }
      })
      .finally(() => {
        if (!cancelled) {
          setLoading(false);
        }
      });
    return () => {
      cancelled = true;
    };
  }, [batchId]);

  return (
    <Drawer
      open={batchId !== null}
      onClose={onClose}
      title={t("tasksCenter.batchDetail.title")}
      styles={{ wrapper: { width: 680 } }}
      destroyOnHidden
    >
      <Spin spinning={loading}>
        {error ? <Alert type="error" showIcon title={error} style={{ marginBottom: 12 }} /> : null}
        {!loading && !error && !data
          ? <Empty image={Empty.PRESENTED_IMAGE_SIMPLE} description={t("tasksCenter.batchDetail.notFound")} />
          : null}
        {data
          ? (
            <Space orientation="vertical" size="middle" style={{ width: "100%" }}>
              <Descriptions
                size="small"
                bordered
                column={{ xs: 1, sm: 2 }}
                items={[
                  { key: "asOf", label: t("replayWorkbench.batch.asOfLabel"), children: data.asOf },
                  {
                    key: "status",
                    label: t("tasksCenter.column.status"),
                    children: <BatchStatusTag status={data.status} />,
                  },
                  {
                    key: "phase",
                    label: t("tasksCenter.batchDetail.phase"),
                    children: (
                      <Tag>
                        {data.phase === "reflection"
                          ? t("replayWorkbench.batch.phaseReflection")
                          : t("replayWorkbench.batch.phaseAnalysis")}
                      </Tag>
                    ),
                  },
                  { key: "batchId", label: "ID", children: <Typography.Text code>{data.batchId}</Typography.Text> },
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
                    key: "picks",
                    label: t("tasksCenter.batchDetail.picks"),
                    children: (
                      <Space size={4} wrap>
                        <Tag color="geekblue">{t("replayWorkbench.batch.recoCount", { n: data.recoPicks ?? 0 })}</Tag>
                        <Tag color="purple">
                          {t("replayWorkbench.batch.serenityCount", { n: data.serenityPicks ?? 0 })}
                        </Tag>
                      </Space>
                    ),
                  },
                  {
                    key: "created",
                    label: t("tasksCenter.history.started"),
                    children: formatDateTime(data.createdAtMs, locale),
                  },
                  {
                    key: "finished",
                    label: t("tasksCenter.batchDetail.finishedAt"),
                    children: formatDateTime(data.finishedAtMs, locale),
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

              {data.serenityError
                ? (
                  <Alert
                    type="warning"
                    showIcon
                    title={t("tasksCenter.batchDetail.serenityError")}
                    description={data.serenityError}
                  />
                )
                : null}
              {data.errorDetail || data.errorCode
                ? (
                  <Alert
                    type="error"
                    showIcon
                    title={data.errorCode ?? t("tasksCenter.batchDetail.errorDetail")}
                    description={data.errorDetail}
                  />
                )
                : null}

              {data.reflectionSummary
                ? (
                  <div>
                    <Typography.Title level={5}>{t("tasksCenter.batchDetail.reflection")}</Typography.Title>
                    <Typography.Paragraph style={{ marginBottom: 0, whiteSpace: "pre-wrap" }}>
                      {data.reflectionSummary}
                    </Typography.Paragraph>
                  </div>
                )
                : null}

              {data.horizons.length > 0
                ? (
                  <div>
                    <Typography.Title level={5}>{t("tasksCenter.batchDetail.horizons")}</Typography.Title>
                    <Table
                      size="small"
                      pagination={false}
                      rowKey="period"
                      columns={horizonColumns}
                      dataSource={data.horizons}
                    />
                  </div>
                )
                : null}

              {data.analyses.length > 0
                ? (
                  <div>
                    <Typography.Title level={5}>
                      {t("replayWorkbench.batch.analysesDetail", { n: data.analyses.length })}
                    </Typography.Title>
                    <Table
                      size="small"
                      pagination={{ pageSize: 10 }}
                      rowKey={(r) => `${r.stockCode}-${r.period}`}
                      columns={analysisColumns}
                      dataSource={data.analyses}
                      scroll={{ y: 360 }}
                    />
                  </div>
                )
                : null}
            </Space>
          )
          : null}
      </Spin>
    </Drawer>
  );
}
