// SPDX-License-Identifier: AGPL-3.0-only

import { horizonLabelKey } from "@/lib/stock-analysis-utils";
import type { TimeTravelBatchAnalysis, TimeTravelHorizonSummary } from "@/types";
import { Space, Tag, Typography } from "antd";
import type { ColumnsType } from "antd/es/table";
import { useTranslation } from "react-i18next";

const { Text } = Typography;

/**
 * 时间旅行批次复盘明细表列定义（实时批次卡与历史详情 Drawer 共用，
 * 禁止在消费方重抄一份）。
 */
export function useBatchAnalysisColumns(): ColumnsType<TimeTravelBatchAnalysis> {
  const { t } = useTranslation();

  return [
    {
      title: t("replayWorkbench.batch.col.stock"),
      dataIndex: "stockCode",
      key: "stock",
      render: (_, r) => (
        <span>
          <Text strong>{r.stockCode}</Text> <Text type="secondary">{r.stockName}</Text>
        </span>
      ),
    },
    {
      title: t("replayWorkbench.batch.col.period"),
      dataIndex: "period",
      key: "period",
      width: 96,
      render: (p: string) => <Tag>{t(horizonLabelKey(p) ?? p)}</Tag>,
    },
    {
      title: t("replayWorkbench.batch.col.confidence"),
      dataIndex: "confidence",
      key: "confidence",
      width: 90,
    },
    {
      title: t("replayWorkbench.batch.col.status"),
      key: "status",
      width: 110,
      render: (_, r) =>
        r.analysisId
          ? <Tag color="success">{t("replayWorkbench.batch.analysisOk")}</Tag>
          : <Tag color="error">{t("replayWorkbench.batch.analysisFail")}</Tag>,
    },
    {
      title: t("replayWorkbench.batch.col.detail"),
      key: "detail",
      ellipsis: true,
      render: (_, r) =>
        r.error
          ? <Text type="danger">{r.error}</Text>
          : <Text type="secondary">{r.analysisId?.slice(0, 8)}</Text>,
    },
  ];
}

/** 四档反思进度表列（horizons：2/5/28/90 交易日档）。 */
export function useBatchHorizonColumns(): ColumnsType<TimeTravelHorizonSummary> {
  const { t } = useTranslation();

  return [
    {
      title: t("replayWorkbench.batch.col.period"),
      dataIndex: "period",
      key: "period",
      render: (p: string) => <Tag>{t(horizonLabelKey(p) ?? p)}</Tag>,
    },
    {
      title: t("replayWorkbench.batch.col.holdingDays"),
      dataIndex: "holdingDays",
      key: "holdingDays",
      width: 80,
    },
    {
      title: t("replayWorkbench.batch.col.hindsight"),
      dataIndex: "hindsightDate",
      key: "hindsightDate",
      width: 120,
    },
    {
      title: t("replayWorkbench.batch.col.due"),
      dataIndex: "due",
      key: "due",
      width: 90,
      render: (due: boolean) =>
        due
          ? <Tag color="blue">{t("replayWorkbench.batch.dueYes")}</Tag>
          : <Tag>{t("replayWorkbench.batch.dueNo")}</Tag>,
    },
    {
      title: t("replayWorkbench.batch.col.reflection"),
      key: "reflection",
      render: (_, h) => (
        <Space size={4}>
          <Tag color="success">{h.completed}</Tag>
          <Tag>{h.pending}</Tag>
          {h.failed > 0 && <Tag color="error">{h.failed}</Tag>}
        </Space>
      ),
    },
  ];
}
