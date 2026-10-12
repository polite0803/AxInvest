// SPDX-License-Identifier: AGPL-3.0-only

import { showBackendError } from "@/lib/errorI18n";
import { useTaskCenterStore } from "@/stores";
import { Button, Card, Empty, List, message, Progress, Space, Statistic, Tag, Typography } from "antd";
import { AlertTriangle, CheckCircle2, Clock4, ListChecks, Loader2 } from "lucide-react";
import { useMemo } from "react";
import { useTranslation } from "react-i18next";
import { getAdapter } from "./adapters/registry";
import { formatDuration } from "./formatUtils";
import type { RunState, UnifiedRun } from "./model";
import { activeRuns, attentionRuns, computeHealth, summarizeWindow } from "./model";
import { RunStatusTag } from "./RunStatusTag";
import { SourceChip } from "./SourceChip";

/**
 * 概览（Temporal Web / Windmill UI 范式）：
 * 健康统计卡 + 需关注 + Live（正在运行）+ 近 24h 汇总。
 */
export function OverviewTab({
  onViewRuns,
}: {
  /** 跳到全部运行 Tab（状态筛选已由健康卡写入 store） */
  onViewRuns: () => void;
}) {
  const { t, i18n } = useTranslation();
  const locale = i18n.language;
  const runs = useTaskCenterStore((s) => s.runs);
  const loading = useTaskCenterStore((s) => s.loading);
  const setFilters = useTaskCenterStore((s) => s.setFilters);
  const selectRun = useTaskCenterStore((s) => s.selectRun);
  const loadRuns = useTaskCenterStore((s) => s.loadRuns);

  const health = useMemo(() => computeHealth(runs), [runs]);
  const attention = useMemo(() => attentionRuns(runs).slice(0, 8), [runs]);
  const live = useMemo(() => activeRuns(runs), [runs]);
  const summary = useMemo(() => {
    const counts = summarizeWindow(runs, 24 * 60 * 60 * 1000);
    return {
      ...counts,
      total: counts.succeeded + counts.failed + counts.canceled,
    };
  }, [runs]);

  const viewByState = (states: RunState[]) => {
    setFilters({ states });
    onViewRuns();
  };

  const handleRetry = async (run: UnifiedRun) => {
    const adapter = getAdapter(run.source);
    if (!adapter?.retry) {
      return;
    }
    try {
      await adapter.retry(run.nativeId);
      message.success(t("tasksCenter.action.retryStarted"));
      await loadRuns(true);
    } catch (e) {
      showBackendError(message, e);
    }
  };

  return (
    <Space direction="vertical" size="middle" style={{ width: "100%" }}>
      {/* 健康统计卡 */}
      <Space wrap size="middle">
        <HealthCard
          color="#1677ff"
          icon={<Loader2 size={18} />}
          label={t("tasksCenter.health.running")}
          value={health.running}
          onClick={() => viewByState(["running"])}
        />
        <HealthCard
          color="#8c8c8c"
          icon={<Clock4 size={18} />}
          label={t("tasksCenter.health.queued")}
          value={health.queued}
          onClick={() => viewByState(["queued"])}
        />
        <HealthCard
          color="#ff4d4f"
          icon={<AlertTriangle size={18} />}
          label={t("tasksCenter.health.failed")}
          value={health.failed}
          onClick={() => viewByState(["failed", "waiting"])}
        />
        <HealthCard
          color="#faad14"
          icon={<Clock4 size={18} />}
          label={t("tasksCenter.health.waiting")}
          value={health.waiting}
          onClick={() => viewByState(["waiting", "paused"])}
        />
      </Space>

      {/* 需关注 */}
      <Card
        size="small"
        title={
          <Space>
            <AlertTriangle size={15} />
            {t("tasksCenter.attention.title")}
          </Space>
        }
        extra={
          <Button type="link" size="small" onClick={() => viewByState(["failed", "waiting"])}>
            {t("tasksCenter.attention.viewAll")}
          </Button>
        }
      >
        {attention.length === 0
          ? (
            <Empty
              image={Empty.PRESENTED_IMAGE_SIMPLE}
              description={t("tasksCenter.attention.empty")}
            />
          )
          : (
            <List
              size="small"
              dataSource={attention}
              renderItem={(run) => (
                <List.Item
                  actions={run.capabilities.retry
                    ? [
                      <Button
                        key="retry"
                        size="small"
                        type="link"
                        onClick={() => void handleRetry(run)}
                      >
                        {t("tasksCenter.action.retry")}
                      </Button>,
                    ]
                    : undefined}
                >
                  <Space size={8}>
                    <RunStatusTag state={run.state} badges={run.badges} />
                    <SourceChip source={run.source} />
                    <Typography.Text>{run.name}</Typography.Text>
                    {run.error?.message && (
                      <Typography.Text type="secondary" style={{ fontSize: 12 }} ellipsis>
                        {run.error.message}
                      </Typography.Text>
                    )}
                  </Space>
                </List.Item>
              )}
            />
          )}
      </Card>

      {/* Live 正在运行 */}
      <Card
        size="small"
        title={
          <Space>
            <Loader2 size={15} className="animate-spin" />
            {t("tasksCenter.live.title")}
          </Space>
        }
      >
        {live.length === 0
          ? (
            <Empty
              image={Empty.PRESENTED_IMAGE_SIMPLE}
              description={t("tasksCenter.live.empty")}
            />
          )
          : (
            <List
              size="small"
              dataSource={live}
              renderItem={(run: UnifiedRun) => (
                <List.Item
                  onClick={() => selectRun(run)}
                  style={{ cursor: "pointer", width: "100%" }}
                >
                  <Space
                    direction="vertical"
                    size={2}
                    style={{ width: "100%" }}
                  >
                    <Space
                      style={{ justifyContent: "space-between", width: "100%" }}
                    >
                      <Space size={8}>
                        <Tag color="processing">
                          {t(`tasksCenter.state.${run.state}`)}
                        </Tag>
                        <SourceChip source={run.source} />
                        <Typography.Text strong>{run.name}</Typography.Text>
                        {run.stage && (
                          <Typography.Text type="secondary">
                            {run.stage}
                          </Typography.Text>
                        )}
                      </Space>
                      <Typography.Text type="secondary" style={{ fontSize: 12 }}>
                        {run.startedAt
                          ? formatDuration(Date.now() - run.startedAt, locale)
                          : "-"}
                      </Typography.Text>
                    </Space>
                    {run.progress !== undefined && <Progress percent={run.progress} size="small" />}
                  </Space>
                </List.Item>
              )}
            />
          )}
      </Card>

      {/* 近 24h 汇总 */}
      <Card size="small" title={t("tasksCenter.summary.title")}>
        <Space wrap size="large">
          <Statistic
            title={t("tasksCenter.summary.succeeded")}
            value={summary.succeeded}
            prefix={<CheckCircle2 size={16} color="#52c41a" />}
          />
          <Statistic
            title={t("tasksCenter.summary.failed")}
            value={summary.failed}
            prefix={<AlertTriangle size={16} color="#ff4d4f" />}
          />
          <Statistic
            title={t("tasksCenter.summary.canceled")}
            value={summary.canceled}
          />
          <Statistic title={t("tasksCenter.summary.total")} value={summary.total} />
        </Space>
        {loading && (
          <Typography.Text type="secondary" style={{ fontSize: 12 }}>
            <ListChecks size={12} /> {t("tasksCenter.runs.refresh")}
          </Typography.Text>
        )}
      </Card>
    </Space>
  );
}

function HealthCard({
  color,
  icon,
  label,
  value,
  onClick,
}: {
  color: string;
  icon: React.ReactNode;
  label: string;
  value: number;
  onClick: () => void;
}) {
  return (
    <Card
      size="small"
      hoverable
      onClick={onClick}
      styles={{ body: { padding: 16, minWidth: 130 } }}
    >
      <Statistic
        title={
          <span style={{ color }}>
            {icon} {label}
          </span>
        }
        value={value}
        valueStyle={{ color }}
      />
    </Card>
  );
}
