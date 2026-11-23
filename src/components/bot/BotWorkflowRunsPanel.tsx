// SPDX-License-Identifier: AGPL-3.0-only

/**
 * P3 编排运行面板（T6'''）。六态徽章、进度、outputs 展开、skipped、取消按钮。
 *
 * 状态以 `effectiveRunStatus` 投影为准：根步骤 completed 但子孙 assignment 仍在途时，
 * run 仍显示 running（ADR-P3-2 ⑤），避免前端提前报完成。
 */

import { effectiveRunStatus, isRunTerminal, runStatusI18nKey, workflowErrorI18nKey } from "@/lib/botWorkflowValidation";
import { backendErrorText } from "@/lib/errorI18n";
import type { BotAssignment, BotWorkflowRun, BotWorkflowRunStatus } from "@/types";
import { App, Button, Card, Empty, Progress, Space, Tag, Typography } from "antd";
import { useState } from "react";
import { useTranslation } from "react-i18next";

const { Text, Paragraph } = Typography;

const STATUS_COLOR: Record<BotWorkflowRunStatus, string> = {
  queued: "default",
  running: "processing",
  waiting: "warning",
  completed: "success",
  failed: "error",
  cancelled: "default",
};

export interface BotWorkflowRunsPanelProps {
  runs: BotWorkflowRun[];
  assignments: BotAssignment[];
  busyRunIds?: string[];
  onCancel: (runId: string) => Promise<void>;
  onViewThread?: (run: BotWorkflowRun) => void;
}

export function BotWorkflowRunsPanel({
  runs,
  assignments,
  busyRunIds = [],
  onCancel,
  onViewThread,
}: BotWorkflowRunsPanelProps) {
  const { t } = useTranslation();
  const { message: antdMessage } = App.useApp();
  const [expanded, setExpanded] = useState<Record<string, boolean>>({});
  const [cancelling, setCancelling] = useState<string | null>(null);

  const handleCancel = async (run: BotWorkflowRun) => {
    setCancelling(run.id);
    try {
      await onCancel(run.id);
    } catch (e) {
      antdMessage.error(backendErrorText(e));
    } finally {
      setCancelling(null);
    }
  };

  if (runs.length === 0) {
    return <Empty data-testid="bot-workflow-runs-empty" description={t("bot.workflowRun.empty")} />;
  }

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 8 }} data-testid="bot-workflow-runs-panel">
      {runs.map((run) => {
        const status = effectiveRunStatus(run, assignments);
        const terminal = isRunTerminal(run, assignments);
        const percent = run.totalSteps > 0 ? Math.floor((run.completedSteps / run.totalSteps) * 100) : 0;
        const outputs = Object.entries(run.context?.outputs ?? {});
        const skipped = run.context?.skipped ?? [];
        const isExpanded = expanded[run.id] === true;
        return (
          <Card
            size="small"
            key={run.id}
            data-testid="bot-workflow-run-item"
            data-status={status}
            title={
              <Space size={6} wrap>
                <Text strong>{run.workflowName}</Text>
                <Tag color={STATUS_COLOR[status]} data-testid="bot-workflow-run-status">
                  {t(runStatusI18nKey(status))}
                </Tag>
                <Text type="secondary" style={{ fontSize: 12 }}>
                  {run.id}
                </Text>
              </Space>
            }
            extra={
              <Space size={4}>
                {onViewThread
                  ? (
                    <Button
                      size="small"
                      type="link"
                      data-testid="bot-workflow-run-view-thread"
                      onClick={() => onViewThread(run)}
                    >
                      {t("bot.workflowRun.viewThread")}
                    </Button>
                  )
                  : null}
                <Button
                  size="small"
                  danger
                  disabled={terminal || busyRunIds.includes(run.id) || cancelling === run.id}
                  loading={cancelling === run.id}
                  data-testid="bot-workflow-run-cancel"
                  onClick={() => {
                    void handleCancel(run);
                  }}
                >
                  {t("bot.workflowRun.cancel")}
                </Button>
              </Space>
            }
          >
            <div data-testid="bot-workflow-run-progress">
              <Progress percent={Math.min(100, Math.max(0, percent))} size="small" />
              <Text type="secondary" style={{ fontSize: 12 }}>
                {`${run.completedSteps}/${run.totalSteps}`}
              </Text>
            </div>
            {run.errorCode
              ? (
                <Paragraph type="danger" style={{ marginBottom: 4 }} data-testid="bot-workflow-run-error">
                  {workflowErrorI18nKey(run.errorCode) ? t(workflowErrorI18nKey(run.errorCode)!) : run.errorCode}
                  {run.errorDetail ? ` ${run.errorDetail}` : ""}
                </Paragraph>
              )
              : null}
            <Button
              type="link"
              size="small"
              data-testid="bot-workflow-run-outputs-toggle"
              onClick={() => setExpanded((prev) => ({ ...prev, [run.id]: !prev[run.id] }))}
            >
              {t("bot.workflowRun.outputs")}
            </Button>
            {isExpanded
              ? (
                <div style={{ marginTop: 4 }} data-testid="bot-workflow-run-outputs">
                  {outputs.length === 0 ? <Text type="secondary">—</Text> : null}
                  {outputs.map(([stepKey, output]) => (
                    <div key={stepKey} style={{ fontSize: 12, marginBottom: 4 }}>
                      <Text strong>{stepKey}</Text>
                      <Text>{output.resultText}</Text>
                    </div>
                  ))}
                  {skipped.length > 0
                    ? (
                      <Text type="secondary" style={{ fontSize: 12 }} data-testid="bot-workflow-run-skipped">
                        {`${t("bot.workflowRun.skipped")}: ${skipped.join(", ")}`}
                      </Text>
                    )
                    : null}
                </div>
              )
              : null}
          </Card>
        );
      })}
    </div>
  );
}
