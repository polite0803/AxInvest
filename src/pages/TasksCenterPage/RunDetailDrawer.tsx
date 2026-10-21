// SPDX-License-Identifier: AGPL-3.0-only

import { translateBackendError } from "@/lib/errorI18n";
import { invoke } from "@/lib/invoke";
import { useTaskCenterStore } from "@/stores";
import type { PipelineRun, TaskExecutionRecord } from "@/types";
import { Drawer, Empty, Spin, Table, Tag, Typography } from "antd";
import type { ColumnsType } from "antd/es/table";
import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { formatDateTime, formatDuration } from "./formatUtils";
import { BatchRunDetailDrawer } from "./jobs/BatchRunDetailDrawer";
import { MoverRecallBatchDrawer } from "./jobs/MoverRecallBatchDrawer";
import { PipelineRunDetailDrawer } from "./jobs/PipelineRunDetailDrawer";
import { StockAnalysisRunDrawer } from "./jobs/StockAnalysisRunDrawer";
import { mapRunTrigger } from "./model";

/**
 * 统一运行详情入口：Runs 表/概览点击任意 run → 按源分发到该源的详情形态。
 *
 * - stock-pipeline → PipelineRunDetailDrawer（逐股明细）
 * - stock-analysis → StockAnalysisRunDrawer（单股分析摘要 + 跳工作台）
 * - time-travel-batch → BatchRunDetailDrawer（analyses/horizons/反思）
 * - mover-recall-analysis → MoverRecallBatchDrawer（逐票 as-of 补分析明细）
 * - scheduled-cron → 该任务的执行历史 Drawer（get_task_execution_history）
 *
 * 后续源（workflow/background/index…）在此扩展，调用方始终只面对 UnifiedRun。
 */
export function RunDetailDrawer() {
  const { t } = useTranslation();
  const selected = useTaskCenterStore((s) => s.selectedRun);
  const selectRun = useTaskCenterStore((s) => s.selectRun);

  const close = useCallback(() => selectRun(null), [selectRun]);

  if (selected?.source === "stock-pipeline") {
    return (
      <PipelineRunDetailDrawer
        run={(selected.raw as PipelineRun | undefined) ?? null}
        onClose={close}
      />
    );
  }

  if (selected?.source === "stock-analysis") {
    return <StockAnalysisRunDrawer run={selected} onClose={close} />;
  }

  if (selected?.source === "time-travel-batch") {
    return <BatchRunDetailDrawer batchId={selected.nativeId} onClose={close} />;
  }

  if (selected?.source === "mover-recall-analysis") {
    return <MoverRecallBatchDrawer batchId={selected.nativeId} onClose={close} />;
  }

  if (selected?.source === "scheduled-cron") {
    return (
      <CronHistoryDrawer
        taskId={selected.nativeId}
        title={selected.name}
        open
        onClose={close}
      />
    );
  }

  // 其余源 P2-2 起接入；未接入时给空抽屉兜底（理论不可见，适配器未注册不会有 run）。
  return (
    <Drawer
      title={selected?.name ?? ""}
      open={selected !== null}
      onClose={close}
      styles={{ wrapper: { width: 560 } }}
    >
      <Empty description={t("tasksCenter.runs.detailUnavailable")} />
    </Drawer>
  );
}

/** cron 任务的执行历史抽屉（与「计划任务」Tab 历史同数据源）。 */
function CronHistoryDrawer({
  taskId,
  title,
  open,
  onClose,
}: {
  taskId: string;
  title: string;
  open: boolean;
  onClose: () => void;
}) {
  const { t, i18n } = useTranslation();
  const locale = i18n.language;
  const [records, setRecords] = useState<TaskExecutionRecord[]>([]);
  const [loading, setLoading] = useState(false);

  useEffect(() => {
    if (!open) {
      return;
    }
    let cancelled = false;
    setLoading(true);
    invoke<TaskExecutionRecord[]>("get_task_execution_history", { taskId })
      .then((rows) => {
        if (!cancelled) {
          setRecords(Array.isArray(rows) ? rows : []);
        }
      })
      .catch(() => {
        if (!cancelled) {
          setRecords([]);
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
  }, [open, taskId]);

  const columns: ColumnsType<TaskExecutionRecord> = [
    {
      title: t("tasksCenter.history.started"),
      dataIndex: "started_at",
      key: "started_at",
      render: (v: number) => (
        <Typography.Text style={{ fontSize: 12 }}>
          {formatDateTime(v, locale)}
        </Typography.Text>
      ),
    },
    {
      title: t("tasksCenter.history.duration"),
      dataIndex: "duration_ms",
      key: "duration_ms",
      width: 90,
      render: (v: number) => (
        <Typography.Text type="secondary" style={{ fontSize: 12 }}>
          {formatDuration(v, locale)}
        </Typography.Text>
      ),
    },
    {
      title: t("tasksCenter.column.trigger"),
      key: "trigger",
      width: 80,
      render: (_: unknown, r: TaskExecutionRecord) => <Tag>{t(`tasksCenter.trigger.${mapRunTrigger(r.trigger)}`)}</Tag>,
    },
    {
      title: t("tasksCenter.history.result"),
      key: "result",
      width: 80,
      // 与计划任务页那套抽屉同判据：success=false 里混着「部分完成」，不能一律显示失败
      render: (_: unknown, r: TaskExecutionRecord) =>
        r.success
          ? <Tag color="success">{t("tasksCenter.lastRun.success")}</Tag>
          : r.partial
          ? <Tag color="warning">{t("tasksCenter.lastRun.partial")}</Tag>
          : <Tag color="error">{t("tasksCenter.lastRun.failed")}</Tag>,
    },
    {
      title: t("tasksCenter.history.detail"),
      key: "detail",
      render: (_: unknown, r: TaskExecutionRecord) =>
        r.error
          ? (
            <Typography.Text type="danger" style={{ fontSize: 12 }}>
              {translateBackendError(r.error)}
            </Typography.Text>
          )
          : r.output
          ? (
            <Typography.Text type="secondary" style={{ fontSize: 12 }}>
              {translateBackendError(r.output)}
            </Typography.Text>
          )
          : "-",
    },
  ];

  return (
    <Drawer
      title={t("tasksCenter.history.title", { name: title })}
      open={open}
      onClose={onClose}
      styles={{ wrapper: { width: 560 } }}
    >
      <Spin spinning={loading}>
        <Table<TaskExecutionRecord>
          rowKey="id"
          size="small"
          columns={columns}
          dataSource={[...records].sort((a, b) => b.started_at - a.started_at)}
          pagination={false}
          locale={{
            emptyText: (
              <Empty
                image={Empty.PRESENTED_IMAGE_SIMPLE}
                description={t("tasksCenter.history.empty")}
              />
            ),
          }}
        />
      </Spin>
    </Drawer>
  );
}
