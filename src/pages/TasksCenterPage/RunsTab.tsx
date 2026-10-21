// SPDX-License-Identifier: AGPL-3.0-only

import { translateBackendError } from "@/lib/errorI18n";
import { useTaskCenterStore } from "@/stores";
import {
  Alert,
  Button,
  Empty,
  Input,
  Progress,
  Segmented,
  Select,
  Space,
  Switch,
  Table,
  type TableColumnsType,
  Tag,
  Typography,
} from "antd";
import { RefreshCw } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { getAdapter, getAllAdapters } from "./adapters/registry";
import { formatDateTime, formatDuration } from "./formatUtils";
import { StartMoverRecallBatchButton } from "./jobs/StartMoverRecallBatchModal";
import type { RunState, RunTrigger, SourceId, UnifiedRun } from "./model";
import { RunStatusTag } from "./RunStatusTag";
import { SourceChip } from "./SourceChip";

/** 有活跃 run 时的兜底轮询间隔（事件订阅在 P2-2 接入）。 */
const ACTIVE_POLL_MS = 2000;

const STATE_KEYS: RunState[] = [
  "queued",
  "running",
  "waiting",
  "paused",
  "succeeded",
  "failed",
  "canceled",
];

const TRIGGER_KEYS: RunTrigger[] = ["schedule", "manual", "system", "external", "unknown"];

/**
 * 全部运行：跨所有任务源的统一执行表（Temporal / Windmill Runs 范式）。
 * 过滤（源/状态/触发/时间窗/搜索）跨源；点行打开统一详情抽屉。
 */
export function RunsTab() {
  const { t, i18n } = useTranslation();
  const locale = i18n.language;

  const runs = useTaskCenterStore((s) => s.runs);
  const sourceErrors = useTaskCenterStore((s) => s.sourceErrors);
  const loading = useTaskCenterStore((s) => s.loading);
  const refreshing = useTaskCenterStore((s) => s.refreshing);
  const filters = useTaskCenterStore((s) => s.filters);
  const setFilter = useTaskCenterStore((s) => s.setFilters);
  const loadRuns = useTaskCenterStore((s) => s.loadRuns);
  const selectRun = useTaskCenterStore((s) => s.selectRun);

  const [autoRefresh, setAutoRefresh] = useState(true);

  // 首次 + 过滤/语言变化时重新聚合
  useEffect(() => {
    void loadRuns(false);
  }, [loadRuns, locale]);

  const hasActive = useMemo(
    () => runs.some((r) => r.state === "running" || r.state === "queued"),
    [runs],
  );

  useEffect(() => {
    if (!autoRefresh || !hasActive) {
      return;
    }
    const timer = window.setInterval(() => void loadRuns(true), ACTIVE_POLL_MS);
    return () => window.clearInterval(timer);
  }, [autoRefresh, hasActive, loadRuns]);

  const sourceOptions = useMemo(
    () => getAllAdapters().map((a) => ({ value: a.id, label: t(a.nameKey) })),
    [t],
  );

  const columns: TableColumnsType<UnifiedRun> = useMemo(
    () => [
      {
        title: t("tasksCenter.column.state"),
        key: "state",
        width: 130,
        render: (_, r) => <RunStatusTag state={r.state} badges={r.badges} />,
      },
      {
        title: t("tasksCenter.column.name"),
        key: "name",
        render: (_, r) => (
          <Space direction="vertical" size={0}>
            <Typography.Text strong>{r.name}</Typography.Text>
            {r.error?.message && (
              <Typography.Text type="danger" style={{ fontSize: 12 }} ellipsis>
                {translateBackendError(r.error.message)}
              </Typography.Text>
            )}
          </Space>
        ),
      },
      {
        title: t("tasksCenter.column.source"),
        key: "source",
        width: 140,
        render: (_, r) => <SourceChip source={r.source} />,
      },
      {
        title: t("tasksCenter.column.trigger"),
        key: "trigger",
        width: 90,
        render: (_, r) => <Tag>{t(`tasksCenter.trigger.${r.trigger}`)}</Tag>,
      },
      {
        title: t("tasksCenter.column.progress"),
        key: "progress",
        width: 170,
        render: (_, r) =>
          r.progress === undefined
            ? r.stage
              ? <Typography.Text type="secondary">{r.stage}</Typography.Text>
              : "-"
            : (
              <Space direction="vertical" size={0} style={{ width: "100%" }}>
                <Progress percent={r.progress} size="small" />
                {r.stage && (
                  <Typography.Text type="secondary" style={{ fontSize: 12 }}>
                    {r.stage}
                  </Typography.Text>
                )}
              </Space>
            ),
      },
      {
        title: t("tasksCenter.column.started"),
        key: "started",
        width: 165,
        render: (_, r) => r.startedAt ? formatDateTime(r.startedAt, locale) : "-",
      },
      {
        title: t("tasksCenter.column.duration"),
        key: "duration",
        width: 90,
        render: (_, r) => {
          if (r.state === "running" && r.startedAt) {
            return formatDuration(Date.now() - r.startedAt, locale);
          }
          return formatDuration(r.durationMs, locale);
        },
      },
      {
        title: t("tasksCenter.column.attempts"),
        key: "attempts",
        width: 70,
        render: (_, r) => r.attempts,
      },
      {
        title: t("tasksCenter.column.actions"),
        key: "actions",
        width: 80,
        render: (_, r) => (
          <Button type="link" size="small" onClick={() => selectRun(r)}>
            {t("tasksCenter.action.detail")}
          </Button>
        ),
      },
    ],
    [t, locale, selectRun],
  );

  return (
    <Space direction="vertical" size="middle" style={{ width: "100%" }}>
      <Space wrap>
        <Input.Search
          allowClear
          size="small"
          style={{ width: 200 }}
          placeholder={t("tasksCenter.filters.search")}
          value={filters.search}
          onChange={(e) => setFilter({ search: e.target.value })}
        />
        <Select
          size="small"
          mode="multiple"
          allowClear
          maxTagCount="responsive"
          style={{ minWidth: 220 }}
          placeholder={t("tasksCenter.filters.sources")}
          value={filters.sources}
          onChange={(vals) => setFilter({ sources: vals as SourceId[] })}
          options={sourceOptions}
        />
        <Select
          size="small"
          mode="multiple"
          allowClear
          style={{ minWidth: 150 }}
          placeholder={t("tasksCenter.filters.states")}
          value={filters.states}
          onChange={(vals) => setFilter({ states: vals as RunState[] })}
          options={STATE_KEYS.map((s) => ({
            value: s,
            label: t(`tasksCenter.state.${s}`),
          }))}
        />
        <Select
          size="small"
          mode="multiple"
          allowClear
          style={{ minWidth: 140 }}
          placeholder={t("tasksCenter.filters.triggers")}
          value={filters.triggers}
          onChange={(vals) => setFilter({ triggers: vals as RunTrigger[] })}
          options={TRIGGER_KEYS.map((k) => ({
            value: k,
            label: t(`tasksCenter.trigger.${k}`),
          }))}
        />
        <Segmented
          size="small"
          value={filters.timeRange}
          onChange={(v) => setFilter({ timeRange: v as typeof filters.timeRange })}
          options={[
            { value: "24h", label: t("tasksCenter.filters.range24h") },
            { value: "7d", label: t("tasksCenter.filters.range7d") },
            { value: "all", label: t("tasksCenter.filters.rangeAll") },
          ]}
        />
      </Space>

      <Space wrap style={{ justifyContent: "space-between", width: "100%" }}>
        <Space size={6}>
          <Typography.Text type="secondary">
            {t("tasksCenter.runs.autoRefresh")}
          </Typography.Text>
          <Switch size="small" checked={autoRefresh} onChange={setAutoRefresh} />
        </Space>
        <Space size={6}>
          <StartMoverRecallBatchButton onStarted={() => void loadRuns(false)} />
          <Button
            size="small"
            icon={
              <RefreshCw
                size={14}
                className={refreshing ? "animate-spin" : undefined}
              />
            }
            onClick={() => void loadRuns(false)}
          >
            {t("tasksCenter.runs.refresh")}
          </Button>
        </Space>
      </Space>

      {sourceErrors.length > 0 && (
        <Alert
          type="warning"
          showIcon
          message={t("tasksCenter.runs.someSourcesFailed")}
          description={sourceErrors
            .map(
              (e) => `${t(getAdapter(e.source)?.nameKey ?? `tasksCenter.source.${e.source}`)}: ${e.message}`,
            )
            .join("；")}
        />
      )}

      <Table<UnifiedRun>
        rowKey="id"
        size="small"
        loading={loading}
        columns={columns}
        dataSource={runs}
        onRow={(r) => ({
          onClick: () => selectRun(r),
          style: { cursor: "pointer" },
        })}
        pagination={{ pageSize: 15, size: "small", showSizeChanger: true }}
        locale={{
          emptyText: (
            <Empty
              image={Empty.PRESENTED_IMAGE_SIMPLE}
              description={t("tasksCenter.runs.empty")}
            />
          ),
        }}
      />
    </Space>
  );
}
