// SPDX-License-Identifier: AGPL-3.0-only

import { translateBackendError } from "@/lib/errorI18n";
import { usePipelineStore } from "@/stores";
import type { PipelineAnalysisSummary, PipelineResult, PipelineRun } from "@/types";
import { Alert, Descriptions, Drawer, Empty, Space, Spin, Table, Tag, Typography } from "antd";
import type { ColumnsType } from "antd/es/table";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { formatDateTime, formatDuration } from "../formatUtils";

interface PipelineRunDetailDrawerProps {
  /** 为空时 Drawer 关闭；切换 run 时按 runId 拉 get_pipeline_run_detail 明细 */
  run: PipelineRun | null;
  onClose: () => void;
}

const STATUS_TAG_COLOR: Record<string, string> = {
  completed: "success",
  success: "success",
  running: "processing",
  failed: "error",
  error: "error",
  cancelled: "default",
};

/**
 * 投资管道历史详情 Drawer：列表行自带 summary（立即可见），
 * 打开后按 runId 拉取逐只股票明细（新分析 / 重估两张表）。
 */
export function PipelineRunDetailDrawer({ run, onClose }: PipelineRunDetailDrawerProps) {
  const { t, i18n } = useTranslation();
  const locale = i18n.language;
  const getRunDetail = usePipelineStore((s) => s.getRunDetail);
  const [detail, setDetail] = useState<PipelineResult | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!run) {
      return;
    }
    let cancelled = false;
    setLoading(true);
    setError(null);
    setDetail(null);
    getRunDetail(run.id)
      .then((result) => {
        if (!cancelled) {
          setDetail(result);
        }
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
  }, [run, getRunDetail]);

  const summary = run?.summary ?? detail?.summary ?? null;
  // 旧版后端对「运行中/失败/从未回写明细」的批次返回 null（2026-10-12 起后端
  // 契约保证空数组，此处保留兜底兼容旧响应），直接读 .length 会触发整页 ErrorBoundary。
  const newAnalyses = detail?.newAnalyses ?? [];
  const reassessed = detail?.reassessed ?? [];

  const stockColumns: ColumnsType<PipelineAnalysisSummary> = [
    {
      title: t("replayWorkbench.batch.col.stock"),
      key: "stock",
      render: (_, r) => (
        <span>
          <Typography.Text strong>{r.stockCode}</Typography.Text>{" "}
          <Typography.Text type="secondary">{r.stockName}</Typography.Text>
        </span>
      ),
    },
    {
      title: t("pipeline.status"),
      dataIndex: "status",
      key: "status",
      width: 100,
      render: (status: string) => <Tag color={STATUS_TAG_COLOR[status?.toLowerCase()] ?? "default"}>{status}</Tag>,
    },
    {
      title: t("tasksCenter.pipelineDetail.action"),
      dataIndex: "action",
      key: "action",
      width: 90,
      render: (action: string | null) => action ?? "-",
    },
    {
      title: t("replayWorkbench.batch.col.confidence"),
      dataIndex: "confidence",
      key: "confidence",
      width: 90,
      render: (confidence: number | null) => confidence ?? "-",
    },
    {
      title: t("replayWorkbench.batch.col.detail"),
      key: "detail",
      ellipsis: true,
      render: (_, r) =>
        r.error
          ? <Typography.Text type="danger">{r.error}</Typography.Text>
          : <Typography.Text type="secondary">{r.analysisId?.slice(0, 8)}</Typography.Text>,
    },
  ];

  return (
    <Drawer
      open={run !== null}
      onClose={onClose}
      title={t("tasksCenter.pipelineDetail.title")}
      styles={{ wrapper: { width: 680 } }}
      destroyOnHidden
    >
      {run
        ? (
          <Spin spinning={loading}>
            {run.errorMessage
              ? <Alert type="error" showIcon title={run.errorMessage} style={{ marginBottom: 12 }} />
              : null}
            {error
              ? (
                <Alert
                  type="warning"
                  showIcon
                  title={t("tasksCenter.pipelineDetail.detailError")}
                  description={error}
                  style={{ marginBottom: 12 }}
                />
              )
              : null}

            <Descriptions
              size="small"
              bordered
              column={{ xs: 1, sm: 2 }}
              items={[
                { key: "runDate", label: t("pipeline.date"), children: run.runDate ?? "-" },
                {
                  key: "asOfDate",
                  label: t("tasksCenter.pipelineDetail.asOfDate"),
                  children: run.asOfDate ?? "-",
                },
                {
                  key: "status",
                  label: t("pipeline.status"),
                  children: <Tag color={STATUS_TAG_COLOR[run.status?.toLowerCase()] ?? "default"}>{run.status}</Tag>,
                },
                { key: "runId", label: "ID", children: <Typography.Text code>{run.id}</Typography.Text> },
                {
                  key: "started",
                  label: t("tasksCenter.history.started"),
                  children: formatDateTime(run.startedAt, locale),
                },
                {
                  key: "completed",
                  label: t("tasksCenter.pipelineDetail.completedAt"),
                  children: formatDateTime(run.completedAt, locale),
                },
                {
                  key: "duration",
                  label: t("tasksCenter.history.duration"),
                  children: run.completedAt !== null
                    ? formatDuration(run.completedAt - run.startedAt, locale)
                    : "-",
                },
                {
                  key: "candidates",
                  label: t("pipeline.candidates"),
                  children: summary?.discovery.candidatesFound ?? 0,
                },
                {
                  key: "newAnalyzed",
                  label: t("tasksCenter.pipelineDetail.newAnalyzed"),
                  children: summary?.analysis.newAnalyzed ?? 0,
                },
                {
                  key: "reassessed",
                  label: t("tasksCenter.pipelineDetail.reassessed"),
                  children: summary?.analysis.reassessed ?? 0,
                },
                {
                  key: "decisions",
                  label: t("tasksCenter.pipelineDetail.decisions"),
                  children: summary
                    ? (
                      <Space size={4} wrap>
                        <Tag color="error">BUY {summary.decisions.buy}</Tag>
                        <Tag color="default">HOLD {summary.decisions.hold}</Tag>
                        <Tag color="processing">WATCH {summary.decisions.watch}</Tag>
                        <Tag color="success">SELL {summary.decisions.sell}</Tag>
                      </Space>
                    )
                    : "-",
                },
                {
                  key: "reflection",
                  label: t("pipeline.reflectionScheduled"),
                  children: summary?.reflectionScheduled ?? 0,
                },
                {
                  key: "note",
                  label: t("tasksCenter.pipelineDetail.note"),
                  children: summary?.note ?? "-",
                  span: 2,
                },
              ]}
            />

            {detail
              ? (
                <Space orientation="vertical" size="middle" style={{ width: "100%", marginTop: 16 }}>
                  <div>
                    <Typography.Title level={5}>
                      {t("tasksCenter.pipelineDetail.newAnalysesList", { n: newAnalyses.length })}
                    </Typography.Title>
                    {newAnalyses.length === 0
                      ? <Empty image={Empty.PRESENTED_IMAGE_SIMPLE} />
                      : (
                        <Table
                          size="small"
                          pagination={{ pageSize: 10 }}
                          rowKey={(r) => r.analysisId ?? `${r.stockCode}-new`}
                          columns={stockColumns}
                          dataSource={newAnalyses}
                        />
                      )}
                  </div>
                  <div>
                    <Typography.Title level={5}>
                      {t("tasksCenter.pipelineDetail.reassessedList", { n: reassessed.length })}
                    </Typography.Title>
                    {reassessed.length === 0
                      ? <Empty image={Empty.PRESENTED_IMAGE_SIMPLE} />
                      : (
                        <Table
                          size="small"
                          pagination={{ pageSize: 10 }}
                          rowKey={(r) => r.analysisId ?? `${r.stockCode}-re`}
                          columns={stockColumns}
                          dataSource={reassessed}
                        />
                      )}
                  </div>
                </Space>
              )
              : null}
          </Spin>
        )
        : null}
    </Drawer>
  );
}
