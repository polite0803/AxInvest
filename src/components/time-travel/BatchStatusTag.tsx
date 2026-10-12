// SPDX-License-Identifier: AGPL-3.0-only

import type { TimeTravelBatchStatus } from "@/types";
import { Tag } from "antd";
import { useTranslation } from "react-i18next";

/**
 * 时间旅行批次复盘状态 Tag（统一色彩语义）：
 * completed=success 绿 / partial=warning 橙 / failed=error 红 /
 * interrupted=default 灰 / running=processing 蓝。
 *
 * 批次卡实时进度、任务中心批次历史表、批次历史详情 Drawer 三处共用，
 * 避免状态 → 颜色/文案映射散落重复。
 */
export function BatchStatusTag({ status }: { status: TimeTravelBatchStatus }) {
  const { t } = useTranslation();

  switch (status) {
    case "completed":
      return <Tag color="success">{t("replayWorkbench.batch.status.completed")}</Tag>;
    case "partial":
      return <Tag color="warning">{t("replayWorkbench.batch.status.partial")}</Tag>;
    case "failed":
      return <Tag color="error">{t("replayWorkbench.batch.status.failed")}</Tag>;
    case "interrupted":
      return <Tag color="default">{t("replayWorkbench.batch.status.interrupted")}</Tag>;
    default:
      return <Tag color="processing">{t("replayWorkbench.batch.status.running")}</Tag>;
  }
}
