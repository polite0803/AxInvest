// SPDX-License-Identifier: AGPL-3.0-only

import { translateBackendError } from "@/lib/errorI18n";
import { invoke } from "@/lib/invoke";
import type { TaskExecutionRecord } from "@/types";
import { Drawer, Empty, Spin, Table, Tag, Typography } from "antd";
import type { ColumnsType } from "antd/es/table";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { formatDateTime, formatDuration } from "../formatUtils";
import { mapRunTrigger } from "../model";

interface CronHistoryDrawerProps {
  taskId: string;
  title: string;
  open: boolean;
  onClose: () => void;
}

/**
 * cron 任务的执行历史抽屉（与「计划任务」Tab 历史同数据源）。
 *
 * 从 `RunDetailDrawer.tsx` 的内部函数移出来（任务 #151）：详情形态现在由
 * `cron.adapter` 自己声明，中心只按注册表分发 ⇒ 这个组件不再需要一个中性文件替它持有。
 */
export function CronHistoryDrawer({ taskId, title, open, onClose }: CronHistoryDrawerProps) {
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
