// SPDX-License-Identifier: AGPL-3.0-only

import { translateBackendError } from "@/lib/errorI18n";
import { invoke } from "@/lib/invoke";
import type { MoverRecallAnalysisRequest } from "@/types";
import { Alert, Button, Form, Input, InputNumber, message, Modal, Space } from "antd";
import { useState } from "react";
import { useTranslation } from "react-i18next";

interface StartMoverRecallBatchModalProps {
  /** 受控开关 */
  open: boolean;
  onClose: () => void;
  /** 批次发起成功后回调（用于立即刷新列表） */
  onStarted: () => void;
}

/**
 * 妖股漏检 → as-of 历史补分析批次发起入口（任务中心「全部运行」手动发起）。
 * 后端 run_mover_recall_analysis 为 fire-and-forget：成功即返回 batchId，明细在抽屉里轮询。
 */
export function StartMoverRecallBatchModal(
  { open, onClose, onStarted }: StartMoverRecallBatchModalProps,
) {
  const { t } = useTranslation();
  const [form] = Form.useForm<MoverRecallAnalysisRequest>();
  const [submitting, setSubmitting] = useState(false);
  const [errorText, setErrorText] = useState<string | null>(null);

  const handleOk = async () => {
    const values = await form.validateFields();
    setSubmitting(true);
    setErrorText(null);
    try {
      const batchId = await invoke<string>("run_mover_recall_analysis", {
        request: {
          batchLimit: values.batchLimit,
          maxUnexplainedShare: values.maxUnexplainedShare,
          runId: values.runId?.trim() ? values.runId.trim() : null,
        } satisfies MoverRecallAnalysisRequest,
      });
      message.success(t("tasksCenter.moverRecall.started", { id: batchId }));
      form.resetFields();
      onStarted();
      onClose();
    } catch (e: unknown) {
      setErrorText(translateBackendError(e));
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <Modal
      title={t("tasksCenter.moverRecall.modalTitle")}
      open={open}
      onCancel={onClose}
      onOk={() => void handleOk()}
      confirmLoading={submitting}
      okText={t("tasksCenter.moverRecall.submit")}
      destroyOnHidden
      forceRender
    >
      <Space direction="vertical" size="middle" style={{ width: "100%" }}>
        <Alert type="info" showIcon message={t("tasksCenter.moverRecall.modalHint")} />
        {errorText ? <Alert type="error" showIcon message={errorText} /> : null}
        <Form
          form={form}
          layout="vertical"
          initialValues={{ batchLimit: 20, maxUnexplainedShare: 0.5 }}
        >
          <Form.Item
            name="batchLimit"
            label={t("tasksCenter.moverRecall.batchLimit")}
            rules={[{ required: true }]}
          >
            <InputNumber min={1} max={100} step={1} precision={0} style={{ width: "100%" }} />
          </Form.Item>
          <Form.Item
            name="maxUnexplainedShare"
            label={t("tasksCenter.moverRecall.maxShare")}
            rules={[{ required: true }]}
            extra={t("tasksCenter.moverRecall.maxShareHint")}
          >
            <InputNumber min={0} max={1} step={0.1} style={{ width: "100%" }} />
          </Form.Item>
          <Form.Item name="runId" label={t("tasksCenter.moverRecall.runId")}>
            <Input allowClear placeholder={t("tasksCenter.moverRecall.runIdPlaceholder")} />
          </Form.Item>
        </Form>
      </Space>
    </Modal>
  );
}

/** 工具栏触发按钮 + Modal 一体封装，RunsTab 零状态接入。 */
export function StartMoverRecallBatchButton({ onStarted }: { onStarted: () => void }) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  return (
    <>
      <Button size="small" type="primary" ghost onClick={() => setOpen(true)}>
        {t("tasksCenter.moverRecall.start")}
      </Button>
      {open
        ? (
          <StartMoverRecallBatchModal
            open={open}
            onClose={() => setOpen(false)}
            onStarted={onStarted}
          />
        )
        : null}
    </>
  );
}
