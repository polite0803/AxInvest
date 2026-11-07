// SPDX-License-Identifier: AGPL-3.0-only

import { useMultiAgentDelegate } from "@/hooks/useMultiAgent";
import { useConversationStore } from "@/stores";
import { App, Input, Modal, Segmented, Space, Typography } from "antd";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

interface DelegateTaskModalProps {
  open: boolean;
  onClose: () => void;
  initialTask: string;
}

export function DelegateTaskModal({
  open,
  onClose,
  initialTask,
}: DelegateTaskModalProps) {
  const { t } = useTranslation();
  const { message: messageApi } = App.useApp();
  const { roles, delegating, fetchRoles, delegateTask: delegateTaskAction } = useMultiAgentDelegate();
  const activeConversation = useConversationStore((s) =>
    s.activeConversationId
      ? s.conversations.find((c) => c.id === s.activeConversationId)
      : undefined
  );
  const [delegateRole, setDelegateRole] = useState("");
  const [delegateTask, setDelegateTask] = useState(initialTask);

  // 打开时同步待委派任务内容，并确保角色已加载
  useEffect(() => {
    if (open) {
      setDelegateTask(initialTask);
      if (roles.length === 0) {
        fetchRoles();
      }
      if (!delegateRole && roles.length > 0) {
        setDelegateRole(roles[0].id);
      }
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, initialTask]);

  return (
    <Modal
      title={t("multiAgent.delegateTitle")}
      open={open}
      onCancel={onClose}
      confirmLoading={delegating}
      onOk={async () => {
        if (!delegateRole || !delegateTask.trim()) {
          messageApi.warning(t("multiAgent.fillRequired"));
          return;
        }
        try {
          await delegateTaskAction({
            roleName: delegateRole,
            task: delegateTask.trim(),
            providerId: activeConversation?.providerId || "",
            modelId: activeConversation?.modelId || "",
          });
          messageApi.success(t("multiAgent.delegateSuccess"));
          onClose();
        } catch (e) {
          messageApi.error(`${t("multiAgent.delegateFailed")}: ${e}`);
        }
      }}
      okText={t("multiAgent.delegateBtn")}
      destroyOnHidden
    >
      <Space orientation="vertical" style={{ width: "100%" }} size="middle">
        <div>
          <Typography.Text type="secondary">{t("multiAgent.selectRole")}</Typography.Text>
          <Segmented
            block
            value={delegateRole}
            onChange={(v) => setDelegateRole(v as string)}
            options={roles.map((r) => ({
              label: r.name,
              value: r.id,
            }))}
          />
        </div>
        <div>
          <Typography.Text type="secondary">{t("multiAgent.taskDescription")}</Typography.Text>
          <Input.TextArea
            value={delegateTask}
            onChange={(e) => setDelegateTask(e.target.value)}
            rows={4}
            placeholder={t("multiAgent.taskPlaceholder")}
          />
        </div>
      </Space>
    </Modal>
  );
}
