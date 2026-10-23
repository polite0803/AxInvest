// SPDX-License-Identifier: AGPL-3.0-only

import { actionKeyOfRecord } from "@/lib/stock-analysis-utils";
import type { AnalysisSummary } from "@/types";
import { Button, Descriptions, Drawer, Space } from "antd";
import { useTranslation } from "react-i18next";
import { useNavigate } from "react-router-dom";
import { formatDateTime, formatDuration } from "../formatUtils";
import type { UnifiedRun } from "../model";
import { RunStatusTag } from "../RunStatusTag";

/**
 * 单股分析运行详情：`raw` 是 `list_stock_analyses` 的 DTO。
 *
 * 列表 DTO 不含 workflowId 与失败原因 ⇒ 这里不做取消/重试；完整报告在工作台，
 * 「打开」按 deepLink 跳 `/stock-analysis/:id`（该路由会重定向到投资工作区）。
 */
export function StockAnalysisRunDrawer({
  run,
  onClose,
}: {
  run: UnifiedRun | null;
  onClose: () => void;
}) {
  const { t, i18n } = useTranslation();
  const navigate = useNavigate();
  const raw = run?.raw as AnalysisSummary | undefined;
  // 后端把空结论下发成 UNAVAILABLE 哨兵，直渲染原值会把内部 token 给用户看 ⇒ 与列表 Tag 同一判据
  const actionKey = raw ? actionKeyOfRecord(raw) : null;

  return (
    <Drawer
      open={run !== null}
      onClose={onClose}
      title={run?.name ?? ""}
      styles={{ wrapper: { width: 560 } }}
      destroyOnHidden
    >
      {run && raw
        ? (
          <Space orientation="vertical" size="middle" style={{ width: "100%" }}>
            <Descriptions
              size="small"
              bordered
              column={1}
              items={[
                {
                  key: "status",
                  label: t("pipeline.status"),
                  children: <RunStatusTag state={run.state} badges={run.badges} />,
                },
                {
                  key: "stock",
                  label: t("replayWorkbench.batch.col.stock"),
                  children: `${raw.stockCode} ${raw.stockName}`,
                },
                {
                  key: "date",
                  label: t("pipeline.date"),
                  children: raw.analysisDate || "-",
                },
                {
                  key: "started",
                  label: t("tasksCenter.history.started"),
                  children: formatDateTime(raw.createdAt, i18n.language),
                },
                {
                  key: "duration",
                  label: t("tasksCenter.history.duration"),
                  children: formatDuration(run.durationMs, i18n.language),
                },
                {
                  key: "action",
                  label: t("tasksCenter.pipelineDetail.action"),
                  children: actionKey ? t(actionKey) : "-",
                },
              ]}
            />
            {run.deepLink
              ? (
                <Button type="primary" onClick={() => navigate(run.deepLink!)}>
                  {t("common.open")}
                </Button>
              )
              : null}
          </Space>
        )
        : null}
    </Drawer>
  );
}
