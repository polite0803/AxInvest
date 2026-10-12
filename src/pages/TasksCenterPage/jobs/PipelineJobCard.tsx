// SPDX-License-Identifier: AGPL-3.0-only

import { JobCard } from "@/components/tasks/JobCard";
import { showBackendError } from "@/lib/errorI18n";
import { invoke, listen } from "@/lib/invoke";
import { usePipelineStore } from "@/stores";
import type { PipelineRun, PipelineStepEvent } from "@/types";
import { Button, Empty, message, Space, Tag, Typography } from "antd";
import { Pause, Play, Workflow } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { formatDateTime, formatRelativeOrAbsolute } from "../formatUtils";

const WORKFLOW_ID = "stock-analysis";

const STATUS_TAG_COLOR: Record<string, string> = {
  completed: "success",
  success: "success",
  running: "processing",
  failed: "error",
  error: "error",
  cancelled: "default",
};

/**
 * 节点状态 → i18n key。**必须显式映射**，不要用 `` t(`pipeline.step${status}`) `` 拼 key ——
 * 拼接型 key 在状态枚举新增时会静默落到未命中分支，且 i18n 硬编码检查无法静态解析。
 * 未登记的状态回退显示原始值（见下方渲染处），不显示空白。
 */
const STEP_STATUS_KEY: Record<string, string> = {
  running: "pipeline.stepRunning",
  completed: "pipeline.stepCompleted",
  failed: "pipeline.stepFailed",
  timeout: "pipeline.stepTimeout",
};

function RunStatusTag({ status }: { status: string }) {
  const color = STATUS_TAG_COLOR[status?.toLowerCase()] ?? "default";
  return <Tag color={color}>{status}</Tag>;
}

/**
 * 手动作业卡 ①：投资管道（发现 → 分析 → 决策 → 反思）。
 *
 * 从旧 `pages/PipelinePage/PipelinePage.tsx` 的内联控制区抽成统一 JobCard：
 * 立即运行 / 暂停 / 恢复操作在头部，运行中步骤流在卡身；
 * 状态条展示最近一次运行与累计成功率。`pipeline-step` 事件监听随挂载/卸载清理。
 */
export function PipelineJobCard() {
  const { t, i18n } = useTranslation();
  const locale = i18n.language;
  const pipelineRuns = usePipelineStore((s) => s.pipelineRuns);
  const isRunning = usePipelineStore((s) => s.isRunning);
  const stepEvents = usePipelineStore((s) => s.stepEvents);
  const fetchHistory = usePipelineStore((s) => s.fetchHistory);
  const runPipeline = usePipelineStore((s) => s.runPipeline);
  const clearStepEvents = usePipelineStore((s) => s.clearStepEvents);
  const addStepEvent = usePipelineStore((s) => s.addStepEvent);
  const [pausing, setPausing] = useState(false);
  const [resuming, setResuming] = useState(false);
  const [isPaused, setIsPaused] = useState(false);

  const handlePause = useCallback(async () => {
    setPausing(true);
    try {
      await invoke("workflow_pause", { workflowId: WORKFLOW_ID });
      message.success(t("chat.workflow.paused"));
      setIsPaused(true);
    } catch (e) {
      showBackendError(message, e, { context: "workflow_pause" });
    } finally {
      setPausing(false);
    }
  }, [t]);

  const handleResume = useCallback(async () => {
    setResuming(true);
    try {
      await invoke("workflow_resume", { workflowId: WORKFLOW_ID });
      message.success(t("chat.workflow.resumed"));
      setIsPaused(false);
    } catch (e) {
      showBackendError(message, e, { context: "workflow_resume" });
    } finally {
      setResuming(false);
    }
  }, [t]);

  useEffect(() => {
    void fetchHistory();
    const unlisten = listen<PipelineStepEvent>("pipeline-step", (event) => {
      addStepEvent(event.payload);
    });
    return () => {
      unlisten.then((fn) => fn());
    };
  }, [fetchHistory, addStepEvent]);

  const handleRun = async () => {
    clearStepEvents();
    setIsPaused(false);
    try {
      await runPipeline();
    } catch (e) {
      showBackendError(message, e, { context: "run_stock_pipeline" });
    }
  };

  const latest: PipelineRun | undefined = pipelineRuns[0];
  const successCount = pipelineRuns.filter((r) => {
    const s = r.status?.toLowerCase();
    return s === "completed" || s === "success";
  }).length;
  const successRate = pipelineRuns.length > 0
    ? Math.round((successCount / pipelineRuns.length) * 100)
    : 0;

  return (
    <JobCard
      testId="job-card-pipeline"
      icon={<Workflow size={16} />}
      title={t("pipeline.title")}
      description={t("tasksCenter.jobs.pipelineDesc")}
      extra={
        <Space>
          {isRunning && (
            <Button
              size="small"
              icon={<Pause size={12} />}
              loading={pausing}
              onClick={handlePause}
            >
              {t("chat.workflow.pause")}
            </Button>
          )}
          {isPaused && (
            <Button
              size="small"
              icon={<Play size={12} />}
              loading={resuming}
              onClick={handleResume}
              className="text-green-500"
            >
              {t("chat.workflow.resume")}
            </Button>
          )}
          <Button type="primary" size="small" loading={isRunning} onClick={handleRun}>
            {isRunning ? t("pipeline.running") : t("pipeline.runNow")}
          </Button>
        </Space>
      }
      status={
        <Space size="small" wrap>
          {isRunning
            ? <Tag color="processing">{t("pipeline.running")}</Tag>
            : latest
            ? (
              <>
                <RunStatusTag status={latest.status} />
                <Typography.Text type="secondary">
                  {t("tasksCenter.jobs.lastRunAt")}: {formatRelativeOrAbsolute(latest.startedAt, locale)}
                </Typography.Text>
              </>
            )
            : <Typography.Text type="secondary">{t("tasksCenter.jobs.noRuns")}</Typography.Text>}
          <Typography.Text type="secondary">
            {t("pipeline.totalRuns")}: {pipelineRuns.length} · {t("pipeline.successRate")}: {successRate}%
          </Typography.Text>
        </Space>
      }
    >
      {isRunning
        ? (
          stepEvents.length === 0
            ? (
              <Empty
                image={Empty.PRESENTED_IMAGE_SIMPLE}
                description={t("pipeline.running")}
              />
            )
            : (
              <ul
                data-testid="pipeline-job-steps"
                style={{ margin: 0, paddingLeft: 20, maxHeight: 220, overflow: "auto" }}
              >
                {stepEvents.map((ev, idx) => {
                  // 文案由前端按 i18n 组装：后端只发 nodeId + status，不再 `format!` 拼中文。
                  // 未知状态 → 显示原始值；nodeId/status 都缺失（旧载荷）→ 回退 detail 兜底。
                  const rawStatus = ev.status ?? "";
                  const statusKey = rawStatus ? STEP_STATUS_KEY[rawStatus] : undefined;
                  const statusText = statusKey
                    ? t(statusKey, { defaultValue: rawStatus })
                    : (rawStatus || (ev.detail ?? ""));
                  return (
                    <li key={`${ev.timestamp}-${idx}`}>
                      <Typography.Text strong>{ev.step}</Typography.Text>
                      <Typography.Text type="secondary">
                        {" — "}
                        {ev.nodeId ? `${ev.nodeId}: ${statusText}` : statusText}
                      </Typography.Text>
                    </li>
                  );
                })}
              </ul>
            )
        )
        : latest?.asOfDate
        ? (
          <Typography.Text type="secondary">
            {t("pipeline.date")}: {latest.runDate} · {formatDateTime(latest.startedAt, locale)}
          </Typography.Text>
        )
        : null}
    </JobCard>
  );
}
